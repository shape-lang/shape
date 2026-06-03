use shape_test::shape_test::ShapeTest;

// =============================================================================
// STAGE Verify — soundness regression-guards (ADR-006 §2.7.30 narrow floor)
//
// The reference-escape→RC promotion flip (§2.7.30.3) makes EXACTLY two loan
// sinks promote: ReturnSlot + ModuleBindingStore. EVERY OTHER reference-escape
// MUST still hard-reject. These tests lock in that each non-flipped guard still
// FIRES at run-level. A regression that silently admits one of these escapes is
// a soundness hole; this module is the tripwire.
//
// NOTE (run-verified 2026-06-03): at branch HEAD the flip is NOT yet
// run-observable — the parallel `escaped_loans` B0003 reject (solver.rs:1147)
// fires alongside the §2.7.30 promotion derivation, and `-> &int` return-type
// unification rejects `return &x`. So the two "flipped" sinks STILL reject too.
// That is a SURFACE on the keystone (the promotion is dead-derived), tracked in
// docs/cluster-audits/. It does NOT weaken these guards — it strengthens them:
// no reference-escape is currently admitted, so no UAF is reachable.
// =============================================================================

// B0004 — `&x` stored into an Array that escapes -> ReferenceStoredInArray.
#[test]
fn guard_b0004_ref_stored_in_array_rejects() {
    ShapeTest::new(
        r#"
        let x = 5
        let arr = [&x]
        arr
    "#,
    )
    .expect_run_err_contains("B0004");
}

// B0004 — `&x` stored into an Object literal -> ReferenceStoredInObject.
#[test]
fn guard_b0004_ref_stored_in_object_rejects() {
    ShapeTest::new(
        r#"
        let x = 5
        let o = { a: &x }
        o
    "#,
    )
    .expect_run_err_contains("B0004");
}

// B0006 — `&mut` reference moved across a structured (`async scope`) task
// boundary via `async let`.
#[test]
fn guard_b0006_mut_ref_across_structured_task_rejects() {
    ShapeTest::new(
        r#"
        async fn test() {
            let mut x = 1
            async let fut = &mut x
            await fut
        }
    "#,
    )
    .expect_run_err_contains("B0006");
}

// B0012 — shared `&` reference sent across a detached task boundary (an
// `async let` outside any `async scope`).
#[test]
fn guard_b0012_shared_ref_across_detached_task_rejects() {
    ShapeTest::new(
        r#"
        async fn test() {
            let x = 1
            async let fut = &x
            await fut
        }
    "#,
    )
    .expect_run_err_contains("B0012");
}

// B0003-closure — a closure capturing `&local` that escapes (the closure is
// returned). ClosureEnv stays hard-rejecting in v0.3.3 (§2.7.30.3).
#[test]
fn guard_b0003_closure_capturing_ref_escapes_rejects() {
    ShapeTest::new(
        r#"
        fn make() {
            let x = 5
            let f = || { &x }
            return f
        }
        make()
    "#,
    )
    .expect_run_err_contains("B0003");
}

// B0001-family — two non-aliased-access params bound to the same variable
// (exclusive-exclusive conflict to the same referent).
#[test]
fn guard_b0001_double_exclusive_same_referent_rejects() {
    ShapeTest::new(
        r#"
        fn take2(&a, &b) { a = b }
        fn test() {
            let mut v = 5
            take2(&v, &v)
        }
        test()
    "#,
    )
    .expect_run_err_contains("B0013");
}

// Flipped-sink CURRENT-STATE lock (ModuleBindingStore): `let r = &x` at module
// scope still rejects at HEAD (parallel escaped_loans reject; promotion derived
// but not run-observable). Locks the current behavior so the keystone's
// suppression-landing flips this test deliberately (red->green) rather than
// silently.
#[test]
fn flipped_sink_module_binding_still_rejects_at_head() {
    ShapeTest::new(
        r#"
        let x = 5
        let r = &x
    "#,
    )
    .expect_run_err_contains("B0003");
}

// Flipped-sink CURRENT-STATE lock (ReturnSlot, bare return): `return &x` from an
// un-annotated fn still rejects at HEAD via the same parallel reject.
#[test]
fn flipped_sink_return_ref_still_rejects_at_head() {
    ShapeTest::new(
        r#"
        fn f() {
            let x = 5
            return &x
        }
        f()
    "#,
    )
    .expect_run_err_contains("cannot return or store a reference");
}

// UAF-probe boundary: the §2.7.30 round-1 UAF shape `fn make() -> &int { let
// x = 5; return &x }` is NOT yet expressible end-to-end — `&int`-typed return
// expressions fail unification (`int is not compatible with &int`) before any
// PromotedCell is built. No live PromotedCell => no UAF reachable. This locks
// that the carrier is unreachable at HEAD (the keystone makes it reachable).
#[test]
fn uaf_probe_typed_return_ref_not_yet_expressible() {
    ShapeTest::new(
        r#"
        fn make() -> &int {
            let x = 5
            return &x
        }
        let r = make()
        r
    "#,
    )
    .expect_run_err_contains("&int");
}
