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
// NOTE (FlipLive, 2026-06-03): the flip is now run-observable. The parallel
// `escaped_loans` B0003 reject (solver.rs) is suppressed for EXACTLY the two
// promoted floor sinks via a `(loan_id, span)` match, and `&expr` infers to
// `&T` so `-> &int` return-type unification succeeds (Borrow-vs-Borrow). The two
// flipped sinks ({ReturnSlot, ModuleBindingStore}) now COMPILE + RUN; every
// OTHER reference-escape below STILL hard-rejects. A referent that is BOTH
// returned AND stored into an escaping container still rejects via the container
// store's own (different-span) `loan_sinks` entry — the suppression cannot reach
// it.
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

// Flipped-sink FLIP lock (ModuleBindingStore): `let r = &x` at module scope now
// COMPILES + RUNS via the §2.7.30 ModuleBindingStore floor sink. `x` is itself a
// program-lifetime module binding, so the reference (a `RefTarget::ModuleBinding`
// carrier) reads through to `5` and outlives every reference to it — the floor
// case where escape→RC promotion is unconditionally sound. FlipLive flipped this
// test red->green deliberately.
#[test]
fn flipped_sink_module_binding_reads_through() {
    ShapeTest::new(
        r#"
        let x = 5
        let r = &x
        print(r)
    "#,
    )
    .expect_output_contains("5");
}

// Flipped-sink FLIP lock (ReturnSlot, bare return): `return &x` from an
// un-annotated fn now compiles + runs via the ReturnSlot floor sink (the parallel
// `escaped_loans` B0003 reject is suppressed for EXACTLY this promoted sink).
#[test]
fn flipped_sink_return_ref_compiles_and_runs() {
    ShapeTest::new(
        r#"
        fn f() -> &int {
            let x = 5
            return &x
        }
        f()
        print("ok")
    "#,
    )
    .expect_output_contains("ok");
}

// UAF-probe on a LIVE PromotedCell: the §2.7.30 round-1 shape
// `fn make() -> &int { let x = 5; return &x }` is now expressible end-to-end.
// `&x` infers to `&int` and unifies against the `-> &int` annotation (Borrow-vs-
// Borrow), the ReturnSlot floor sink promotes the referent to a `SharedCow`
// PromotedCell, and reading the returned reference after the def-site frame has
// popped reads the live value `5` through the owning `Arc<SharedCell>` share — NO
// use-after-free (the owning share keeps the referent alive past frame-pop).
#[test]
fn uaf_probe_typed_return_ref_reads_live_promoted_cell() {
    ShapeTest::new(
        r#"
        fn make() -> &int {
            let x = 5
            return &x
        }
        let r = make()
        print(r)
    "#,
    )
    .expect_run_ok();
}
