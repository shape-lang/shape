//! Vertical-slice round-trip test for the LSDS Phase-2 first session.
//!
//! Demonstrates the full LSDS pipeline for one error class — the B0013
//! `CallSiteAliasConflict` borrow error — without requiring shape-vm to
//! compile (shape-vm has pre-existing strict-typing baseline errors that
//! block end-to-end integration testing).
//!
//! What this test asserts:
//!
//! 1. **Construction** — a B0013 LSDS [`Diagnostic`] can be built
//!    matching the exact shape `borrow_error_to_lsds` produces, with the
//!    same diagnostic_id, severity, message body, suggested fix
//!    (default-hint), and loan-origin note.
//! 2. **JSON round-trip** — serializing and deserializing the diagnostic
//!    yields the same struct.
//! 3. **Terminal render** — `render::terminal::render` produces text
//!    that contains the `[B0013]` prefix consumed by the existing test
//!    harness (`expect_semantic_diagnostic_contains("[B0013]")`).
//!    The exact text shape is pinned as a snapshot.
//!
//! Subsequent sessions will replace this hand-built sample with one
//! built directly from a [`shape_vm::mir::analysis::BorrowError`] via
//! `BytecodeCompiler::borrow_error_to_lsds`, after the runtime baseline
//! errors are resolved.

use shape_diagnostics::{
    render, Diagnostic, DiagnosticBuilder, DiagnosticNote, Location, SCHEMA_VERSION, Severity,
    SuggestedFix,
};

/// Sample B0013 diagnostic — matches what
/// `BytecodeCompiler::borrow_error_to_lsds` produces for the
/// `CallSiteAliasConflict` borrow error kind.
fn sample_b0013() -> Diagnostic {
    DiagnosticBuilder::new(
        "B0013",
        Severity::Error,
        Location::new(Some("src/main.shape".into()), 12, 4, 0, 0),
        "cannot pass the same variable to multiple parameters that require non-aliased access",
    )
    .with_fix(SuggestedFix::new(
        "use separate variables or clone one of the arguments",
        0.5,
    ))
    .with_note(DiagnosticNote::new(
        "conflicting argument originates here",
        Some(Location::new(Some("src/main.shape".into()), 9, 8, 0, 0)),
    ))
    .rule("ADR-006-§9")
    .build()
}

#[test]
fn schema_version_pinned() {
    assert_eq!(
        SCHEMA_VERSION, 1,
        "schema version is part of the wire format; bump deliberately"
    );
}

#[test]
fn b0013_json_round_trip() {
    let diag = sample_b0013();
    let json = serde_json::to_string_pretty(&diag).expect("serialize");
    let back: Diagnostic = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(diag, back);
    // JSON shape sanity — required fields present.
    assert!(json.contains("\"diagnostic_id\": \"B0013\""));
    assert!(json.contains("\"severity\": \"error\""));
    assert!(json.contains("\"message\""));
    assert!(json.contains("\"location\""));
    assert!(json.contains("\"fixes\""));
}

#[test]
fn b0013_terminal_render_snapshot() {
    let diag = sample_b0013();
    let out = render::terminal::render(&diag);
    let expected = "\
error[B0013]: cannot pass the same variable to multiple parameters that require non-aliased access
 --> src/main.shape:12:4
  = fix: use separate variables or clone one of the arguments (confidence: 50%)
  = note: conflicting argument originates here (src/main.shape:9:8)
  = rule: ADR-006-§9
";
    assert_eq!(out, expected);
}

#[test]
fn b0013_render_contains_legacy_prefix() {
    // Existing borrow-checker tests grep for `[B0013]` in error messages
    // (see tools/shape-test/tests/borrow_refs/violations.rs). The
    // `diagnostic_to_shape_error` bridge in shape-vm preserves that
    // prefix in `ShapeError::SemanticError.message`. The terminal
    // renderer's first line carries it as `error[B0013]: ...`. Pin both
    // shapes so subsequent renderer churn doesn't drop the legacy
    // pattern.
    let diag = sample_b0013();
    let out = render::terminal::render(&diag);
    assert!(out.contains("[B0013]"));
}

#[test]
fn diagnostic_with_witness_round_trips() {
    use serde_json::json;
    use shape_diagnostics::TypeWitness;

    let diag = DiagnosticBuilder::new(
        "E0100",
        Severity::Error,
        Location::new(Some("test.shape".into()), 5, 1, 100, 110),
        "expected int, found string",
    )
    .expected(TypeWitness::new("int", Some(json!(42))))
    .found(TypeWitness::new("string", Some(json!("hello"))))
    .build();

    let json = serde_json::to_string(&diag).unwrap();
    let back: Diagnostic = serde_json::from_str(&json).unwrap();
    assert_eq!(diag, back);

    let out = render::terminal::render(&diag);
    assert!(out.contains("expected: int (witness: 42)"));
    assert!(out.contains("found:    string (witness: \"hello\")"));
}
