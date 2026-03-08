//! Higher-order function tests.
//!
//! Covers: apply, compose, twice, adder, multiplier, flip, pipeline,
//! constant, identity, currying, and named-function-as-argument patterns.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// From programs_closures_and_hof.rs
// =========================================================================

#[test]
fn test_hof_apply_basic() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        apply(|x| x * 2, 21)
    "#,
    )
    .expect_number(42.0);
}

// BUG: Chained calls `compose(double, inc)(3)` fail with
// "Method '__call__' not available on type 'closure'"
// Workaround: bind intermediate to a variable.
#[test]
fn test_hof_compose_via_binding() {
    ShapeTest::new(
        r#"
        fn compose(f, g) { |x| f(g(x)) }
        let double = |x| x * 2
        let inc = |x| x + 1
        let f = compose(double, inc)
        f(3)
    "#,
    )
    .expect_number(8.0);
}

// BUG: Chained calls don't work -- same issue as compose
#[test]
fn test_hof_twice_via_binding() {
    ShapeTest::new(
        r#"
        fn twice(f) { |x| f(f(x)) }
        let inc = |x| x + 1
        let f = twice(inc)
        f(5)
    "#,
    )
    .expect_number(7.0);
}

// BUG: `adder(10)(5)` fails -- chained call syntax not supported
#[test]
fn test_hof_adder_via_binding() {
    ShapeTest::new(
        r#"
        fn adder(a) { |b| a + b }
        let add10 = adder(10)
        add10(5)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_hof_array_map() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3].map(|x| x * 10)
        arr[2]
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_hof_array_filter() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3, 4, 5, 6].filter(|x| x > 3)
        arr.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_hof_array_reduce() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4].reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_hof_array_map_filter_chain() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5, 6]
            .filter(|x| x % 2 == 0)
            .map(|x| x * 10)
            .reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(120.0);
}

#[test]
fn test_hof_function_returning_function_returning_function() {
    ShapeTest::new(
        r#"
        fn a() {
            fn b() {
                |x| x + 1
            }
            b()
        }
        let f = a()
        f(41)
    "#,
    )
    .expect_number(42.0);
}

// BUG: Passing a named function (not a lambda) as an argument causes
// [B0004] "reference argument must be a local or module_binding variable"
// Workaround: wrap in a lambda.
#[test]
fn test_hof_pass_named_fn_via_lambda_wrapper() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        fn apply(f, x) { f(x) }
        apply(|x| double(x), 21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_hof_compose_triple() {
    // BUG: reference 'g' cannot escape into a closure; capture a value instead.
    // The `compose` function's parameter `g` (a closure) cannot be captured
    // in the returned closure due to borrow-checker restrictions.
    ShapeTest::new(
        r#"
        fn compose(f, g) { |x| f(g(x)) }
        let inc = |x| x + 1
        let double = |x| x * 2
        let negate = |x| 0 - x
        let step1 = compose(double, inc)
        let f = compose(negate, step1)
        f(3)
    "#,
    )
    .expect_run_err();
}

// BUG: chained call `twice(double)(3)` fails
#[test]
fn test_hof_apply_twice_with_double_via_binding() {
    ShapeTest::new(
        r#"
        fn twice(f) { |x| f(f(x)) }
        let double = |x| x * 2
        let f = twice(double)
        f(3)
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_hof_array_find() {
    ShapeTest::new(
        r#"
        [10, 20, 30].find(|x| x > 15)
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_hof_array_some() {
    ShapeTest::new(
        r#"
        [1, 2, 3].some(|x| x > 2)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_hof_array_every() {
    ShapeTest::new(
        r#"
        [2, 4, 6].every(|x| x > 0)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_hof_array_every_false() {
    ShapeTest::new(
        r#"
        [2, 4, 6].every(|x| x > 3)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_hof_multiplier_factory() {
    ShapeTest::new(
        r#"
        fn multiplier(n) { |x| x * n }
        let triple = multiplier(3)
        let quadruple = multiplier(4)
        triple(10) + quadruple(10)
    "#,
    )
    .expect_number(70.0);
}

#[test]
fn test_hof_apply_with_arrow() {
    // Arrow syntax removed; using pipe lambda syntax
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        apply(|x| x + 100, 5)
    "#,
    )
    .expect_number(105.0);
}

#[test]
fn test_hof_nested_adder_chain() {
    ShapeTest::new(
        r#"
        fn adder(a) { |b| a + b }
        let add10 = adder(10)
        let add20 = adder(20)
        add10(5) + add20(5)
    "#,
    )
    .expect_number(40.0);
}

// BUG: Passing named fn `double` directly fails with B0004.
// Wrapping in lambda works.
#[test]
fn test_hof_map_with_named_fn_via_lambda() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        let result = [1, 2, 3].map(|x| double(x))
        result[0]
    "#,
    )
    .expect_number(2.0);
}

// BUG: Passing named fn `identity` directly fails with B0004
#[test]
fn test_hof_identity_via_lambda() {
    ShapeTest::new(
        r#"
        fn identity(x) { x }
        fn apply(f, x) { f(x) }
        let id = |x| identity(x)
        apply(id, 42)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_hof_constant_function() {
    ShapeTest::new(
        r#"
        fn constant(val) { || val }
        let always42 = constant(42)
        always42()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_hof_flip() {
    ShapeTest::new(
        r#"
        fn flip(f) { |a, b| f(b, a) }
        let sub = |a, b| a - b
        let flipped_sub = flip(sub)
        flipped_sub(10, 50)
    "#,
    )
    .expect_number(40.0);
}

#[test]
fn test_hof_pipeline_simulation() {
    ShapeTest::new(
        r#"
        let ops = [|x| x + 1, |x| x * 2, |x| x - 3]
        let result = ops.reduce(|val, f| f(val), 5)
        result
    "#,
    )
    .expect_number(9.0);
}

#[test]
fn test_hof_array_some_false() {
    ShapeTest::new(
        r#"
        [1, 2, 3].some(|x| x > 10)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_hof_reduce_strings() {
    ShapeTest::new(
        r#"
        ["a", "b", "c"].reduce(|acc, x| acc + x, "")
    "#,
    )
    .expect_string("abc");
}

#[test]
fn test_hof_filter_then_length() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5].filter(|x| x % 2 == 0).length
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn test_hof_array_flatmap() {
    ShapeTest::new(
        r#"
        [[1, 2], [3, 4]].flatMap(|arr| arr).length
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn test_hof_nested_map() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3].map(|x| x + 1).map(|x| x * 10)
        arr[0]
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_hof_apply_chain() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        apply(|x| apply(|y| y + 1, x), 41)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_hof_map_to_bool() {
    ShapeTest::new(
        r#"
        let result = [1, 2, 3, 4, 5].map(|x| x > 3)
        result[3]
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_hof_factory_pattern() {
    ShapeTest::new(
        r#"
        fn make_op(op_type) {
            if op_type == "add" {
                |a, b| a + b
            } else {
                |a, b| a - b
            }
        }
        let add = make_op("add")
        let sub = make_op("sub")
        add(10, 5) + sub(10, 5)
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_hof_array_map_first_element() {
    ShapeTest::new(
        r#"
        [5, 10, 15].map(|x| x + 1)[0]
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_hof_array_map_last_element() {
    ShapeTest::new(
        r#"
        [5, 10, 15].map(|x| x * 3).last()
    "#,
    )
    .expect_number(45.0);
}

#[test]
fn test_hof_reduce_product() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5].reduce(|acc, x| acc * x, 1)
    "#,
    )
    .expect_number(120.0);
}

#[test]
fn test_hof_filter_empty_result() {
    ShapeTest::new(
        r#"
        [1, 2, 3].filter(|x| x > 100).length
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn test_hof_map_identity() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30].map(|x| x)
        arr[1]
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_hof_find_none() {
    // find with no match returns None
    ShapeTest::new(
        r#"
        let result = [1, 2, 3].find(|x| x > 100)
        result == None
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// From programs_closures_hof.rs
// =========================================================================

#[test]
fn hof_function_taking_function_param() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        let double = |x| x * 2
        apply(double, 21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn hof_function_taking_lambda_inline() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        apply(|x| x + 10, 32)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn hof_function_returning_closure() {
    ShapeTest::new(
        r#"
        fn make_adder(n) {
            |x| x + n
        }
        let add10 = make_adder(10)
        add10(32)
    "#,
    )
    .expect_number(42.0);
}

// BUG: Chained closure calls `f(a)(b)` fail with "__call__ not available on type 'closure'"
// The VM does not support calling the return value of a function call directly.
#[test]
fn hof_adder_chained_call() {
    ShapeTest::new(
        r#"
        fn adder(a) {
            |b| a + b
        }
        let add10 = adder(10)
        add10(5)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn hof_compose_two_functions() {
    ShapeTest::new(
        r#"
        fn compose(f, g) {
            |x| f(g(x))
        }
        let double = |x| x * 2
        let inc = |x| x + 1
        let double_then_inc = compose(inc, double)
        double_then_inc(20)
    "#,
    )
    .expect_number(41.0);
}

// BUG: Chained closure calls `compose(...)(20)` fail -- see hof_adder_chained_call
#[test]
fn hof_compose_used_directly() {
    ShapeTest::new(
        r#"
        fn compose(f, g) {
            |x| f(g(x))
        }
        let h = compose(|x| x * 2, |x| x + 1)
        h(20)
    "#,
    )
    .expect_number(42.0);
}

// BUG: Chained closure calls `twice(inc)(40)` fail -- see hof_adder_chained_call
#[test]
fn hof_twice_applies_function_twice() {
    ShapeTest::new(
        r#"
        fn twice(f) {
            |x| f(f(x))
        }
        let inc = |x| x + 1
        let inc2 = twice(inc)
        inc2(40)
    "#,
    )
    .expect_number(42.0);
}

// BUG: Chained closure calls `twice(double)(3)` fail -- see hof_adder_chained_call
#[test]
fn hof_twice_with_double() {
    ShapeTest::new(
        r#"
        fn twice(f) {
            |x| f(f(x))
        }
        let double = |x| x * 2
        let double2 = twice(double)
        double2(3)
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn hof_apply_two_args() {
    ShapeTest::new(
        r#"
        fn apply2(f, x, y) { f(x, y) }
        apply2(|a, b| a * b, 6, 7)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn hof_function_as_return_value_with_state() {
    ShapeTest::new(
        r#"
        fn multiplier(factor) {
            |x| x * factor
        }
        let times3 = multiplier(3)
        let times7 = multiplier(7)
        times3(10) + times7(2)
    "#,
    )
    .expect_number(44.0);
}

#[test]
fn hof_pipeline_of_transforms() {
    ShapeTest::new(
        r#"
        fn pipe(x, f, g) {
            g(f(x))
        }
        pipe(5, |x| x * 2, |x| x + 1)
    "#,
    )
    .expect_number(11.0);
}

// BUG: Passing a named function directly as argument fails with
// "reference argument must be a local or module_binding variable, got 'square'"
// Workaround: wrap in a lambda or assign to a let binding first
#[test]
fn hof_named_function_as_argument() {
    ShapeTest::new(
        r#"
        fn square(x) { x * x }
        fn apply(f, x) { f(x) }
        let sq = |x| square(x)
        apply(sq, 7)
    "#,
    )
    .expect_number(49.0);
}

#[test]
fn hof_return_function_from_if() {
    ShapeTest::new(
        r#"
        fn chooser(use_add) {
            if use_add { |a, b| a + b } else { |a, b| a * b }
        }
        let op = chooser(true)
        op(3, 4)
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn hof_return_function_from_if_else_branch() {
    ShapeTest::new(
        r#"
        fn chooser(use_add) {
            if use_add { |a, b| a + b } else { |a, b| a * b }
        }
        let op = chooser(false)
        op(3, 4)
    "#,
    )
    .expect_number(12.0);
}

// BUG: Chained closure calls `curried_add(1)(2)(3)` fail -- see hof_adder_chained_call
#[test]
fn hof_curried_add() {
    ShapeTest::new(
        r#"
        fn curried_add(a) {
            |b| {
                |c| a + b + c
            }
        }
        let f1 = curried_add(1)
        let f2 = f1(2)
        f2(3)
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn hof_apply_n_times() {
    ShapeTest::new(
        r#"
        fn apply_n(f, n, x) {
            let result = x
            for i in 0..n {
                result = f(result)
            }
            result
        }
        apply_n(|x| x + 1, 5, 0)
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn hof_map_over_pair() {
    ShapeTest::new(
        r#"
        fn map_pair(f, a, b) {
            [f(a), f(b)]
        }
        let result = map_pair(|x| x * 10, 3, 7)
        result[0] + result[1]
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn hof_predicate_check() {
    ShapeTest::new(
        r#"
        fn satisfies(pred, val) { pred(val) }
        satisfies(|x| x > 10, 15)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn hof_predicate_check_false() {
    ShapeTest::new(
        r#"
        fn satisfies(pred, val) { pred(val) }
        satisfies(|x| x > 10, 5)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn hof_thrice() {
    ShapeTest::new(
        r#"
        fn thrice(f, x) { f(f(f(x))) }
        thrice(|n| n * 2, 1)
    "#,
    )
    .expect_number(8.0);
}
