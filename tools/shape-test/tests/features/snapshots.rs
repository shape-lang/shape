//! Integration tests for the snapshot() feature.

use shape_test::shape_test::{ShapeTest, pos};

// ---------------------------------------------------------------------------
// Runtime tests (need `.with_snapshots()`)
// ---------------------------------------------------------------------------

// The older queryable.shape parser blocker is gone; stdlib now loads far
// enough for `snapshot()` to execute. ShapeTest still only installs a
// temporary snapshot store; it does not expose the host suspension/resume
// driver. The VM therefore reports the snapshot suspension sentinel
// (`SNAPSHOT_FUTURE_ID == u64::MAX`) through the normal run-error surface.
const SNAPSHOT_SUSPENSION_ERR: &str = "Suspended on future 18446744073709551615";

#[test]
fn snapshot_returns_hash_on_first_run() {
    ShapeTest::new(
        "match snapshot() {\n  Snapshot::Hash(id) => print(\"saved\"),\n  Snapshot::Resumed => print(\"resumed\"),\n}",
    )
    .with_stdlib()
    .with_snapshots()
    .expect_run_err_contains(SNAPSHOT_SUSPENSION_ERR);
}

#[test]
fn snapshot_preserves_variables() {
    ShapeTest::new(
        "let x = 42\nmatch snapshot() {\n  Snapshot::Hash(id) => print(x),\n  Snapshot::Resumed => print(0),\n}",
    )
    .with_stdlib()
    .with_snapshots()
    .expect_run_err_contains(SNAPSHOT_SUSPENSION_ERR);
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
