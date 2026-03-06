//! Integration tests for the snapshot() feature.

use shape_test::shape_test::{ShapeTest, pos};

// ---------------------------------------------------------------------------
// Runtime tests (need `.with_snapshots()`)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_returns_hash_on_first_run() {
    // snapshot() suspends, engine creates snapshot, resumes with Snapshot::Hash(id)
    ShapeTest::new(
        "match snapshot() {\n  Snapshot::Hash(id) => print(\"saved\"),\n  Snapshot::Resumed => print(\"resumed\"),\n}",
    )
    .with_stdlib()
    .with_snapshots()
    .expect_output("saved");
}

#[test]
fn snapshot_preserves_variables() {
    // Variables defined before snapshot() are accessible after it returns
    ShapeTest::new(
        "let x = 42\nmatch snapshot() {\n  Snapshot::Hash(id) => print(x),\n  Snapshot::Resumed => print(0),\n}",
    )
    .with_stdlib()
    .with_snapshots()
    .expect_output("42");
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
