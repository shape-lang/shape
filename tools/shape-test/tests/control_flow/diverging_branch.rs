//! Diverging-branch exclusion from if/else + match-arm expression-type
//! inference (strict-flip, 2026-06-22).
//!
//! A branch that DIVERGES — its body ends in / is dominated by a
//! `return`/`break`/`continue` — is the NEVER/bottom type. It produces no
//! value of any ordinary type and must be EXCLUDED from the if/else (and
//! match-arm) expression-type unification: only the NON-diverging branches
//! unify to form the expression type. If ALL branches diverge, the construct
//! is Never.
//!
//! ROOT repro: `if v > 0 { acc = v } else { return Err("neg") }` previously
//! unified the diverging `else` (typed as the fn's `Result<…>` return) against
//! the void `then`, wrongly rejecting with "void is not compatible with
//! Result<…>". The fix excludes the diverging else; the if-statement is void,
//! the tail `Ok(acc)` is the Result return.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// ROOT repro — diverging else inside a void-then if, Result return
// =========================================================================

/// parse(5): then-branch runs, tail `Ok(acc)` returns 5.
#[test]
fn diverging_else_ok_path() {
    ShapeTest::new(
        r#"
        fn parse(v: int) -> Result<int, string> {
            var acc = 0
            if v > 0 { acc = v } else { return Err("neg") }
            Ok(acc)
        }
        match parse(5) { Ok(n) => n, Err(e) => -1 }
    "#,
    )
    .expect_number(5.0);
}

/// parse(-1): diverging else fires, returns Err("neg").
#[test]
fn diverging_else_err_path() {
    ShapeTest::new(
        r#"
        fn parse(v: int) -> Result<int, string> {
            var acc = 0
            if v > 0 { acc = v } else { return Err("neg") }
            Ok(acc)
        }
        match parse(-1) { Ok(n) => "ok", Err(e) => e }
    "#,
    )
    .expect_string("neg");
}

// =========================================================================
// Diverging-then: the if-expression type is the (non-diverging) else type
// =========================================================================

/// `if c { return X } else { y }` — then diverges, so the if-expression type
/// is `y`'s type (string here). When c is false the value is the else branch.
#[test]
fn diverging_then_yields_else_type() {
    ShapeTest::new(
        r#"
        fn f(c: bool) -> string {
            let y = if c { return "early" } else { "yval" }
            y
        }
        f(false)
    "#,
    )
    .expect_string("yval");
}

/// Same fn, c true: the diverging then fires.
#[test]
fn diverging_then_take_diverging_branch() {
    ShapeTest::new(
        r#"
        fn f(c: bool) -> string {
            let y = if c { return "early" } else { "yval" }
            y
        }
        f(true)
    "#,
    )
    .expect_string("early");
}

// =========================================================================
// Diverging-else with valued then: the if-expression type is the then type
// =========================================================================

/// `if c { a } else { return X }` — else diverges, so the if-expression type
/// is `a`'s type (int here).
#[test]
fn diverging_else_yields_then_type() {
    ShapeTest::new(
        r#"
        fn g(c: bool) -> int {
            let a = if c { 7 } else { return 99 }
            a
        }
        g(true)
    "#,
    )
    .expect_number(7.0);
}

// =========================================================================
// Match: one arm returns, others valued — the valued arms' type wins
// =========================================================================

/// `match o { Some(n) => n, None => return 0 }` — the `None` arm diverges and
/// is excluded; the match type is the valued `Some` arm's `int`.
#[test]
fn match_one_arm_returns_valued_arms_win() {
    ShapeTest::new(
        r#"
        fn h(o: Option<int>) -> int {
            let x = match o { Some(n) => n, None => return 0 }
            x + 1
        }
        h(Some(5))
    "#,
    )
    .expect_number(6.0);
}

/// Same fn, None: the diverging arm fires.
#[test]
fn match_one_arm_returns_take_diverging() {
    ShapeTest::new(
        r#"
        fn h(o: Option<int>) -> int {
            let x = match o { Some(n) => n, None => return 0 }
            x + 1
        }
        h(None)
    "#,
    )
    .expect_number(0.0);
}

// =========================================================================
// All-diverge → the construct is Never (used in a diverging position)
// =========================================================================

/// Both branches diverge; the if itself is Never. The function returns via
/// whichever branch fires.
#[test]
fn all_branches_diverge_is_never() {
    ShapeTest::new(
        r#"
        fn k(c: bool) -> int {
            if c { return 1 } else { return 2 }
        }
        k(true)
    "#,
    )
    .expect_number(1.0);
}

// =========================================================================
// Regressions: divergence exclusion must NOT relax ordinary unification
// =========================================================================

/// A NORMAL if/else (no divergence) still unifies BOTH branches.
#[test]
fn normal_if_else_still_unifies() {
    ShapeTest::new(
        r#"
        fn ok(c: bool) -> int {
            let x = if c { 1 } else { 2 }
            x
        }
        ok(false)
    "#,
    )
    .expect_number(2.0);
}

/// A genuine type mismatch between two NON-diverging branches still REJECTS —
/// the Never exclusion only applies to diverging branches.
#[test]
fn normal_mismatch_still_rejected() {
    ShapeTest::new(
        r#"
        fn bad(c: bool) -> int {
            let x = if c { 1 } else { "s" }
            99
        }
        bad(true)
    "#,
    )
    .expect_run_err();
}
