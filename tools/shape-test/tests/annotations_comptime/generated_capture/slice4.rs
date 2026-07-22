//! ADR-009 C1 slice 4: public generated-capture execution and mutation proofs.

use shape_test::shape_test::ShapeTest;

fn expect_number_in_both_tiers(source: &str, expected: f64) {
    ShapeTest::new(source).expect_number(expected);
    ShapeTest::new(source).with_jit().expect_number(expected);
}

fn expect_string_in_both_tiers(source: &str, expected: &str) {
    ShapeTest::new(source).expect_string(expected);
    ShapeTest::new(source).with_jit().expect_string(expected);
}

fn expect_error_in_both_tiers(source: &str, expected: &str) {
    ShapeTest::new(source).expect_run_err_contains(expected);
    ShapeTest::new(source)
        .with_jit()
        .expect_run_err_contains(expected);
}

const MOVE_LET: &str = r#"
annotation add_answer() on type {
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { let base = 40
        let worker = |; move base| base + 2
        worker() }
    }
  }
}
@add_answer()
type Job { id: int }
let job = Job { id: 1 }
job.answer()
"#;

const MOVE_LET_MUT: &str = r#"
annotation add_answer() on type {
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { let mut total = 40
        let worker = |; move total| { total = total + 2
          total }
        worker() }
    }
  }
}
@add_answer()
type Job { id: int }
let job = Job { id: 1 }
job.answer()
"#;

const MOVE_HEAP_LET: &str = r#"
annotation add_label() on type {
  comptime post(target, ctx) {
    extend target {
      method label() -> string { let label = "shape"
        let worker = |; move label| label
        worker() }
    }
  }
}
@add_label()
type Job { id: int }
let job = Job { id: 1 }
job.label()
"#;

const NESTED_SHARE: &str = r#"
annotation add_answer() on type {
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { var total = 40
        let outer = |; share total| {
          let inner = |step: int; share total| { total = total + step
            total }
          inner(2)
          total }
        outer() }
    }
  }
}
@add_answer()
type Job { id: int }
let job = Job { id: 1 }
job.answer()
"#;

/// Exact #53 regression: ordinary inferred closures must preserve the same
/// SharedCell through a nested recapture. This is intentionally separate from
/// the generated explicit-`share` case above so the underlying inferred path
/// cannot regress behind the C1 declaration surface.
const ORDINARY_NESTED_SHARE: &str = r#"
fn answer() -> int {
  var total = 40
  let outer = || {
    let inner = || {
      total = total + 2
      total
    }
    inner()
    total
  }
  outer()
}
answer()
"#;

const ESCAPING_OWNED_MUTABLE: &str = r#"
fn make_counter() {
  let mut total = 40
  return || {
    total = total + 2
    total
  }
}
make_counter()
"#;

#[test]
fn generated_move_over_plain_local_runs_in_both_tiers() {
    expect_number_in_both_tiers(MOVE_LET, 42.0);
}

#[test]
fn generated_move_over_owned_mutable_local_runs_in_both_tiers() {
    expect_number_in_both_tiers(MOVE_LET_MUT, 42.0);
}

#[test]
fn generated_move_of_heap_local_runs_in_both_tiers() {
    expect_string_in_both_tiers(MOVE_HEAP_LET, "shape");
}

#[test]
fn generated_nested_share_recaptures_the_same_cell_in_both_tiers() {
    expect_number_in_both_tiers(NESTED_SHARE, 42.0);
}

#[test]
fn ordinary_inferred_nested_share_recaptures_the_same_cell_in_both_tiers() {
    expect_number_in_both_tiers(ORDINARY_NESTED_SHARE, 42.0);
}

/// Mutation control for the identity proof above: redirecting the inner
/// closure to an intentionally fresh Shared cell leaves the outer holder at
/// 40. This pins that the positive result comes from cell identity, not merely
/// from returning the inner closure's updated snapshot.
#[test]
fn nested_share_fresh_cell_control_breaks_outer_observation() {
    let isolated = NESTED_SHARE.replace(
        "let inner = |step: int; share total| { total = total + step\n            total }",
        "var isolated = total\n          let inner = |step: int; share isolated| { isolated = isolated + step\n            isolated }",
    );
    assert_ne!(
        isolated, NESTED_SHARE,
        "the fresh-cell mutation must actually rewrite the inner closure — guard against a \
         no-op .replace after the 5b-2 direct-form re-indent left the control vacuous"
    );
    expect_number_in_both_tiers(&isolated, 40.0);
}

#[test]
fn escaping_owned_mutable_still_hits_the_structural_veto_in_both_tiers() {
    expect_error_in_both_tiers(
        ESCAPING_OWNED_MUTABLE,
        "[B0003] mutable binding 'total' cannot be captured by an escaping closure",
    );
}

/// Mutation proof for the accepted `move` × local-let clause. Deleting the
/// clause must hit the generated implicit-capture gate; changing its mode must
/// hit C0908. If inference were still authoritative, either mutation could
/// accidentally keep the positive test green.
#[test]
fn move_let_accept_rejects_deleted_or_changed_clause() {
    let deleted = MOVE_LET.replace("|; move base|", "||");
    expect_error_in_both_tiers(
        &deleted,
        "generated closure implicitly captures 'base'; generated captures must be explicit",
    );

    let changed = MOVE_LET.replace("|; move base|", "|; share base|");
    expect_error_in_both_tiers(
        &changed,
        "[C0908] 'base' is not a shared-ownership binding; use `move base`",
    );
}

/// Same mutation pair for the accepted OwnedMutable lowering. The body writes
/// through the capture, so this also proves a deleted declaration cannot ride
/// ordinary mutation-based inference in generated code.
#[test]
fn move_let_mut_accept_rejects_deleted_or_changed_clause() {
    let deleted = MOVE_LET_MUT.replace("|; move total|", "||");
    expect_error_in_both_tiers(
        &deleted,
        "generated closure implicitly captures 'total'; generated captures must be explicit",
    );

    let changed = MOVE_LET_MUT.replace("|; move total|", "|; share total|");
    expect_error_in_both_tiers(
        &changed,
        "[C0908] 'total' is not a shared-ownership binding; use `move total`",
    );
}

/// Heap values use the same declared `move` authority, but are pinned
/// separately because their emitted layout owns a refcounted share.
#[test]
fn move_heap_accept_rejects_deleted_or_changed_clause() {
    let deleted = MOVE_HEAP_LET.replace("|; move label|", "||");
    expect_error_in_both_tiers(
        &deleted,
        "generated closure implicitly captures 'label'; generated captures must be explicit",
    );

    let changed = MOVE_HEAP_LET.replace("|; move label|", "|; share label|");
    expect_error_in_both_tiers(
        &changed,
        "[C0908] 'label' is not a shared-ownership binding; use `move label`",
    );
}

/// Mutate both levels independently. The outer deletion/change proves the
/// declaring `var` is explicit; the inner pair is the #53 regression guard —
/// inherited Shared storage must not be silently inferred or un-shared.
#[test]
fn nested_share_accept_rejects_deleted_or_changed_clauses() {
    let outer_deleted = NESTED_SHARE.replace("|; share total|", "||");
    expect_error_in_both_tiers(
        &outer_deleted,
        "generated closure implicitly captures 'total'; generated captures must be explicit",
    );

    let outer_changed = NESTED_SHARE.replace("|; share total|", "|; move total|");
    expect_error_in_both_tiers(
        &outer_changed,
        "[C0904] 'total' is a shared-ownership binding and cannot be un-shared",
    );

    let inner_deleted = NESTED_SHARE.replace("|step: int; share total|", "|step: int|");
    expect_error_in_both_tiers(
        &inner_deleted,
        "generated closure implicitly captures 'total'; generated captures must be explicit",
    );

    let inner_changed = NESTED_SHARE.replace("|step: int; share total|", "|step: int; move total|");
    expect_error_in_both_tiers(
        &inner_changed,
        "[C0904] 'total' is a shared-ownership binding and cannot be un-shared",
    );
}
