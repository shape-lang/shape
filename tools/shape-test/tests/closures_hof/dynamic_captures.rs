//! Tests for closures with many captures (>16) to verify the JIT's dynamic
//! capture allocation path, and mutable capture correctness in the interpreter.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Many-capture closures (tests dynamic capture allocation in JIT)
// =========================================================================

#[test]
fn closure_20_captures_all_used() {
    // Create 20 local variables and capture them all in a single closure
    ShapeTest::new(
        r#"
        let a = 1
        let b = 2
        let c = 3
        let d = 4
        let e = 5
        let f = 6
        let g = 7
        let h = 8
        let i = 9
        let j = 10
        let k = 11
        let l = 12
        let m = 13
        let n = 14
        let o = 15
        let p = 16
        let q = 17
        let r = 18
        let s = 19
        let t = 20
        let sum_all = || a + b + c + d + e + f + g + h + i + j + k + l + m + n + o + p + q + r + s + t
        sum_all()
    "#,
    )
    .expect_number(210.0);
}

#[test]
fn closure_many_captures_with_params() {
    // Closure that captures many variables AND takes parameters
    ShapeTest::new(
        r#"
        let v1 = 1
        let v2 = 2
        let v3 = 3
        let v4 = 4
        let v5 = 5
        let v6 = 6
        let v7 = 7
        let v8 = 8
        let v9 = 9
        let v10 = 10
        let v11 = 11
        let v12 = 12
        let v13 = 13
        let v14 = 14
        let v15 = 15
        let v16 = 16
        let v17 = 17
        let v18 = 18
        let compute = |x, y| v1 + v2 + v3 + v4 + v5 + v6 + v7 + v8 + v9 + v10 + v11 + v12 + v13 + v14 + v15 + v16 + v17 + v18 + x + y
        compute(100, 200)
    "#,
    )
    .expect_number(471.0); // sum(1..18) + 100 + 200 = 171 + 300 = 471
}

#[test]
fn closure_captures_mixed_types() {
    // Closure capturing numbers, strings, and booleans
    ShapeTest::new(
        r#"
        let n1 = 1
        let n2 = 2
        let n3 = 3
        let s1 = "hello"
        let s2 = " "
        let s3 = "world"
        let b1 = true
        let n4 = 4
        let n5 = 5
        let n6 = 6
        let n7 = 7
        let n8 = 8
        let n9 = 9
        let n10 = 10
        let n11 = 11
        let n12 = 12
        let n13 = 13
        let n14 = 14
        let n15 = 15
        let n16 = 16
        let n17 = 17
        let get_str = || s1 + s2 + s3
        get_str()
    "#,
    )
    .expect_string("hello world");
}

// =========================================================================
// Mutable capture propagation to enclosing scope
// =========================================================================

#[test]
fn mutable_capture_modifies_enclosing_scope() {
    ShapeTest::new(
        r#"
        let mut x = 0
        let set = |v| { x = v }
        set(42)
        x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn mutable_capture_counter_reads_from_outer() {
    ShapeTest::new(
        r#"
        let mut count = 0
        let inc = || { count = count + 1 }
        inc()
        inc()
        inc()
        count
    "#,
    )
    .expect_number(3.0);
}

// =========================================================================
// Factory pattern with captures (returned closures)
// =========================================================================

#[test]
fn closure_factory_with_many_captures() {
    // make_adder captures `base` from its parameter
    ShapeTest::new(
        r#"
        fn make_adder(base) {
            |x| base + x
        }
        let add10 = make_adder(10)
        let add20 = make_adder(20)
        add10(5) + add20(5)
    "#,
    )
    .expect_number(40.0); // (10+5) + (20+5) = 40
}
