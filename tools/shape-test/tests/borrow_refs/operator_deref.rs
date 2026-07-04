use shape_test::shape_test::ShapeTest;

// =============================================================================
// Operator auto-deref of first-class references (finding 9, ADR-006 §2.7.30)
//
// `let r = &n; r + 1` reads THROUGH the reference, mirroring the already
// auto-derefing method dispatch (`r.len()`) and the book example in
// `advanced/ownership-deep-dive.mdx` ("First-Class References" —
// `let val = r + 1` "reads through r via DerefLoad"). Before the fix the
// operator-operand path checked the `Borrow{int}` type directly and rejected
// with "Borrow{int} does not implement Numeric".
// =============================================================================

#[test]
fn ref_shared_arithmetic_auto_derefs() {
    // The verbatim ownership-deep-dive.mdx "First-Class References" example.
    ShapeTest::new(
        r#"
        let x = 42
        let r = &x
        let val = r + 1
        val
    "#,
    )
    .expect_number(43.0);
}

#[test]
fn ref_shared_arithmetic_in_function() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let n = 5
            let r = &n
            let v = r + 1
            return v
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn ref_mut_arithmetic_auto_derefs() {
    // `&mut` parity: an exclusive reference auto-derefs the same way.
    ShapeTest::new(
        r#"
        fn test() -> int {
            let mut n = 5
            let r = &mut n
            let v = r + 1
            return v
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn ref_number_arithmetic_preserves_family() {
    // `number` referent: no int<->number coercion is introduced; `&number`
    // derefs to `number` and stays `number`.
    ShapeTest::new(
        r#"
        fn test() -> number {
            let n = 2.5
            let r = &n
            return r + 1.0
        }
        test()
    "#,
    )
    .expect_number(3.5);
}

#[test]
fn ref_unary_neg_auto_derefs() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let n = 5
            let r = &n
            return -r
        }
        test()
    "#,
    )
    .expect_number(-5.0);
}

#[test]
fn ref_comparison_auto_derefs() {
    ShapeTest::new(
        r#"
        fn test() -> bool {
            let n = 5
            let r = &n
            return r > 3
        }
        test()
    "#,
    )
    .expect_bool(true);
}

#[test]
fn ref_method_dispatch_still_works() {
    // Regression guard: the pre-existing `r.len()` method auto-deref that the
    // operator path now mirrors must keep working.
    ShapeTest::new(
        r#"
        fn test() -> int {
            let s = "hello"
            let r = &s
            return r.len()
        }
        test()
    "#,
    )
    .expect_number(5.0);
}
