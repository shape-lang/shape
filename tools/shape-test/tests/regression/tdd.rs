use shape_test::shape_test::ShapeTest;

// BUG-1: Bare Some/None patterns should work on untyped variables
#[test]
fn bug1_bare_none_match() {
    ShapeTest::new(
        r#"
        let x = Some(42)
        match x { Some(v) => v, None => -1 }
    "#,
    )
    .expect_number(42.0);
}

// BUG-2: Chained calls f(a)(b)
#[test]
fn bug2_chained_call() {
    ShapeTest::new(
        r#"
        fn adder(a) { |b| a + b }
        adder(10)(5)
    "#,
    )
    .expect_number(15.0);
}

// BUG-3: Mutable capture propagates to outer scope
#[test]
fn bug3_mutable_capture_propagates() {
    ShapeTest::new(
        r#"
        let count = 0
        let inc = || { count = count + 1; count }
        inc()
        inc()
        count
    "#,
    )
    .expect_number(2.0);
}

// BUG-4: Module member access via ShapeEngine
#[test]
fn bug4_module_member_access() {
    ShapeTest::new(
        r#"
        mod math { pub fn add(a, b) { a + b } }
        math.add(1, 2)
    "#,
    )
    .expect_number(3.0);
}

// BUG-5: Named fn as HOF argument
#[test]
fn bug5_named_fn_as_argument() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        fn apply(f, x) { f(x) }
        apply(double, 21)
    "#,
    )
    .expect_number(42.0);
}

// BUG-6: Closure captures fn-local let variables
#[test]
fn bug6_closure_captures_local_let() {
    ShapeTest::new(
        r#"
        fn make_adder() {
            let base = 10
            let f = |x| base + x
            f
        }
        let f = make_adder()
        f(5)
    "#,
    )
    .expect_number(15.0);
}

// BUG-7: Grandparent scope capture
#[test]
fn bug7_grandparent_capture() {
    ShapeTest::new(
        r#"
        let x = 100
        let outer = || {
            |y| x + y
        }
        let f = outer()
        f(42)
    "#,
    )
    .expect_number(142.0);
}

// BUG-8: break expr propagates loop value
#[test]
fn bug8_break_value() {
    ShapeTest::new(
        r#"
        let result = loop { break 42 }
        result
    "#,
    )
    .expect_number(42.0);
}

// BUG-9: const compound assignment errors
#[test]
fn bug9_const_compound_assign() {
    ShapeTest::new(
        r#"
        const C = 10
        C += 1
    "#,
    )
    .expect_run_err_contains("const");
}

// BUG-10: Nested struct field mutation persists
#[test]
fn bug10_nested_field_mutation() {
    ShapeTest::new(
        r#"
        type Inner { val: int }
        type Outer { data: Inner }
        let o = Outer { data: Inner { val: 1 } }
        o.data.val = 42
        o.data.val
    "#,
    )
    .expect_number(42.0);
}

// BUG-11: push through & ref propagates
#[test]
fn bug11_push_through_ref() {
    ShapeTest::new(
        r#"
        fn add_item(&arr, item) { arr = arr.push(item) }
        var items = []
        add_item(&items, 1)
        add_item(&items, 2)
        items.length
    "#,
    )
    .expect_number(2.0);
}

// BUG-12: const passed by exclusive & ref errors
#[test]
fn bug12_const_exclusive_ref() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        const C = 5
        inc(&C)
    "#,
    )
    .expect_run_err_contains("const");
}

// BUG-13: Field names don't collide with builtins
#[test]
fn bug13_field_name_sum() {
    ShapeTest::new(
        r#"
        type Stats { sum: number, product: number }
        let s = Stats { sum: 10.0, product: 20.0 }
        s.sum + s.product
    "#,
    )
    .expect_number(30.0);
}

// BUG-14: Object destructuring in match
#[test]
fn bug14_object_destructure_match() {
    ShapeTest::new(
        r#"
        let p = { x: 5, y: 3 }
        match p {
            {x, y} where x > y => x - y,
            _ => 0
        }
    "#,
    )
    .expect_number(2.0);
}

// BUG-15: let in ref fn copies, not aliases
#[test]
fn bug15_let_copies_in_ref_fn() {
    ShapeTest::new(
        r#"
        fn swap(&a, &b) {
            let old = a
            a = b
            b = old
        }
        var x = 1
        var y = 2
        swap(&x, &y)
        x * 10 + y
    "#,
    )
    .expect_number(21.0);
}
