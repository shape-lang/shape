//! T1 KEYSTONE (strict-flip, 2026-06-22): post-solve per-expression type table.
//!
//! The type-inference engine records the RESOLVED type of every expression
//! keyed by source span; `BytecodeCompiler::infer_expr_type` consults that
//! table FIRST (before the per-context patch ladder). This clears the recurring
//! static-type-erasure class at its ROOT: a function-body / module-scope local
//! whose type comes from a collection-dispatch result (`.map`/`.filter`/`.pop`),
//! a match-arm binder, or a method result now reaches the downstream binop/
//! comparison use site as a concrete type instead of erasing to `unknown`.
//!
//! Before this fix each program below failed strict typing with
//! `operand types are 'unknown' and 'int'`.

use shape_test::shape_test::ShapeTest;

// c1: `.map` result into a for-in then a comparison. The element type of
// `sals` (= `int`, the `salary` field type) must reach `v > mx`.
#[test]
fn keystone_map_result_into_for_in_comparison() {
    ShapeTest::new(
        r#"
        let roster = [{salary: 100}, {salary: 200}]
        let sals = roster.map(|e| e.salary)
        var mx = 0
        for v in sals {
            if v > mx { mx = v }
        }
        mx
    "#,
    )
    .expect_number(200.0);
}

// c2: a match-arm binder into arithmetic. `Some(n) => n + 1` — `n`'s type must
// reach the `+ 1` use site. We construct the `Option` explicitly so the test
// asserts the TYPE-table behavior (the binder reaches arithmetic), independent
// of any HashMap.get runtime quirk.
#[test]
fn keystone_match_arm_binder_into_arithmetic() {
    ShapeTest::new(
        r#"
        let opt = Some(1)
        let r = match opt {
            Some(n) => n + 1,
            None => 0
        }
        r
    "#,
    )
    .expect_number(2.0);
}

// `.filter` result into arithmetic accumulation.
#[test]
fn keystone_filter_result_into_arithmetic() {
    ShapeTest::new(
        r#"
        let xs = [1, 2, 3, 4]
        let evens = xs.filter(|x| x % 2 == 0)
        var s = 0
        for v in evens {
            s = s + v
        }
        s
    "#,
    )
    .expect_number(6.0);
}

// `.pop()` result into arithmetic.
#[test]
fn keystone_pop_result_into_arithmetic() {
    ShapeTest::new(
        r#"
        var xs = [10, 20, 30]
        let last = xs.pop()
        last + 5
    "#,
    )
    .expect_number(35.0);
}

// PREVIOUSLY-PATCHED-CONTEXT regression: scalar-element (`.first()`) into a
// binop still works (the per-context R3-elemerasure patch remains the FALLBACK
// for any expression the table misses; here the table also covers it — either
// route must keep producing the proven `int`).
#[test]
fn keystone_first_result_into_arithmetic_still_works() {
    ShapeTest::new(
        r#"
        let a = [10, 20, 30]
        let x = a.first()
        x + 1
    "#,
    )
    .expect_number(11.0);
}

// PREVIOUSLY-PATCHED-CONTEXT regression: user-function return type
// (Phase-3e) into a binop still works.
#[test]
fn keystone_function_return_into_arithmetic_still_works() {
    ShapeTest::new(
        r#"
        fn dbl(n: int) -> int { n * 2 }
        dbl(5) + dbl(7)
    "#,
    )
    .expect_number(24.0);
}
