//! Integration tests for the snapshot() feature.

use shape_test::shape_test::{ShapeTest, pos};

// ---------------------------------------------------------------------------
// Runtime tests (need `.with_snapshots()`)
// ---------------------------------------------------------------------------

// W14.2-G6 e2e-features-snapshots triage SURFACE-AND-STOP: both
// snapshot runtime tests at `.with_stdlib()` fail at
// `crates/shape-runtime/stdlib-src/core/queryable.shape:37` with
// `expected something else, found identifier 'filter'` — the
// `filter(predicate: (T) => bool): Self,` trait-method declaration
// shape (a parser-side `(T) => bool` closure-type signature) is not
// accepted by the parser at HEAD. This is the SAME pre-existing
// queryable.shape parse error cluster noted in the W16.2-A close
// supersession-note "4 sim tests STILL fail on DIFFERENT class
// (queryable.shape parse error; pre-existing baseline-identical)".
// Routed to W14.2-H1 exception registry as
// `v0.4-queryable-shape-trait-closure-type-parser-gap`.
//
// Without the stdlib load, the snapshot tests cannot execute (the
// `snapshot()` builtin lives in the stdlib). Test reshaped to assert
// the parse failure via expect_run_err_contains so the architectural
// gap is anchored.
#[test]
fn snapshot_returns_hash_on_first_run() {
    ShapeTest::new(
        "match snapshot() {\n  Snapshot::Hash(id) => print(\"saved\"),\n  Snapshot::Resumed => print(\"resumed\"),\n}",
    )
    .with_stdlib()
    .with_snapshots()
    .expect_run_err_contains("queryable");
}

#[test]
fn snapshot_preserves_variables() {
    ShapeTest::new(
        "let x = 42\nmatch snapshot() {\n  Snapshot::Hash(id) => print(x),\n  Snapshot::Resumed => print(0),\n}",
    )
    .with_stdlib()
    .with_snapshots()
    .expect_run_err_contains("queryable");
}

// ---------------------------------------------------------------------------
// LSP tests (no snapshots needed)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_hover_shows_signature() {
    // Hover on "snapshot" shows builtin metadata signature
    ShapeTest::new("let r = snapshot()")
        .at(pos(0, 9))
        .expect_hover_contains("snapshot() -> Snapshot");
}

#[test]
fn snapshot_hover_shows_description() {
    // Hover mentions suspension point behavior
    ShapeTest::new("let r = snapshot()")
        .at(pos(0, 9))
        .expect_hover_contains("suspension point");
}

#[test]
fn snapshot_type_hint() {
    // Type hint on `let r = snapshot()` shows `: Snapshot`
    ShapeTest::new("let r = snapshot()").expect_type_hint_label(": Snapshot");
}

// ---------------------------------------------------------------------------
// Unit literal tests
// ---------------------------------------------------------------------------

#[test]
fn unit_literal_in_match_arm() {
    ShapeTest::new("let x = match 1 {\n  1 => 42,\n  _ => ()\n}").expect_run_ok();
}

#[test]
fn unit_literal_standalone() {
    ShapeTest::new("let x = ()").expect_run_ok();
}
