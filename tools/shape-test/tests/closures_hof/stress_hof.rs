//! Stress tests for higher-order functions.

use shape_test::shape_test::ShapeTest;

/// Verifies lambda modulo.
#[test]
fn test_lambda_modulo() {
    ShapeTest::new(
        r#"
        let f = |x| x % 3
        [f(7), f(9), f(10)]
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies named fn as map arg.
#[test]
fn test_named_fn_as_map_arg() {
    ShapeTest::new(
        r#"
        fn double(x) { return x * 2 }
        [1, 2, 3].map(double)
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies named fn as filter arg.
#[test]
fn test_named_fn_as_filter_arg() {
    ShapeTest::new(
        r#"
        fn test() {
            fn is_even(x) { return x % 2 == 0 }
            return [1, 2, 3, 4, 5, 6].filter(is_even).length
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

/// Verifies take while.
#[test]
fn test_take_while() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].takeWhile(|x| x < 4)
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies skip while.
#[test]
fn test_skip_while() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].skipWhile(|x| x < 3)
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies group by even odd runs without error.
#[test]
fn test_group_by_even_odd() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5, 6].groupBy(|x| x % 2)
    "#,
    )
    .expect_run_ok();
}

/// Verifies lambda returns string.
#[test]
fn test_lambda_returns_string() {
    ShapeTest::new(
        r#"
        let greet = |name| "hello"
        greet("world")
    "#,
    )
    .expect_string("hello");
}

/// Verifies reduce count positives.
#[test]
fn test_reduce_count_positives() {
    ShapeTest::new(
        r#"
        [-1, 2, -3, 4, 5].reduce(|acc, x| if x > 0 { acc + 1 } else { acc }, 0)
    "#,
    )
    .expect_number(3.0);
}

/// Verifies reduce string concat.
#[test]
fn test_reduce_string_concat() {
    ShapeTest::new(
        r#"
        ["a", "b", "c"].reduce(|acc, x| acc + x, "")
    "#,
    )
    .expect_string("abc");
}

/// Verifies closure does not leak locals.
#[test]
fn test_closure_does_not_leak_locals() {
    ShapeTest::new(
        r#"
        fn test() {
            let f = |x| {
                let temp = x * 2
                temp + 1
            }
            return f(5)
        }
        test()
    "#,
    )
    .expect_number(11.0);
}

/// Verifies multiple closures same scope.
#[test]
fn test_multiple_closures_same_scope() {
    ShapeTest::new(
        r#"
        fn make_ops(n) {
            let add = |x| x + n
            let mul = |x| x * n
            return [add(10), mul(10)]
        }
        make_ops(5)
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies large array map.
#[test]
fn test_large_array_map() {
    ShapeTest::new(
        r#"
        fn test() {
            let arr = []
            let i = 0
            while i < 100 {
                arr.push(i);
                i = i + 1
            }
            return arr.map(|x| x * 2).length
        }
        test()
    "#,
    )
    .expect_number(100.0);
}

/// Verifies large array filter.
#[test]
fn test_large_array_filter() {
    ShapeTest::new(
        r#"
        fn test() {
            let arr = []
            let i = 0
            while i < 100 {
                arr.push(i);
                i = i + 1
            }
            return arr.filter(|x| x % 2 == 0).length
        }
        test()
    "#,
    )
    .expect_number(50.0);
}

/// Verifies closure reuse.
#[test]
fn test_closure_reuse() {
    ShapeTest::new(
        r#"
        let f = |x| x * x
        [f(1), f(2), f(3), f(4)]
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies closure in loop.
#[test]
fn test_closure_in_loop() {
    ShapeTest::new(
        r#"
        fn test() {
            let f = |x| x * 2
            let sum = 0
            let i = 0
            while i < 5 {
                sum = sum + f(i)
                i = i + 1
            }
            return sum
        }
        test()
    "#,
    )
    .expect_number(20.0);
}

/// Verifies closure capture bool.
#[test]
fn test_closure_capture_bool() {
    ShapeTest::new(
        r#"
        let flag = true
        let check = |x| if flag { x * 2 } else { x }
        check(5)
    "#,
    )
    .expect_number(10.0);
}

/// Verifies lambda comparison lt.
#[test]
fn test_lambda_comparison_lt() {
    ShapeTest::new(
        r#"(
        [5, 3, 8, 1, 4].filter(|x| x < 4)
    ).length"#,
    )
    .expect_number(2.0);
}

/// Verifies lambda comparison lte.
#[test]
fn test_lambda_comparison_lte() {
    ShapeTest::new(
        r#"(
        [5, 3, 8, 1, 4].filter(|x| x <= 4)
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies lambda comparison gte.
#[test]
fn test_lambda_comparison_gte() {
    ShapeTest::new(
        r#"(
        [5, 3, 8, 1, 4].filter(|x| x >= 5)
    ).length"#,
    )
    .expect_number(2.0);
}

/// Verifies lambda comparison eq.
#[test]
fn test_lambda_comparison_eq() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 2, 1].filter(|x| x == 2)
    ).length"#,
    )
    .expect_number(2.0);
}

/// Verifies lambda comparison neq.
#[test]
fn test_lambda_comparison_neq() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 2, 1].filter(|x| x != 2)
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies lambda logical and.
#[test]
fn test_lambda_logical_and() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].filter(|x| x > 3 && x < 8)
    ).length"#,
    )
    .expect_number(4.0);
}

/// Verifies lambda logical or.
#[test]
fn test_lambda_logical_or() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].filter(|x| x == 1 || x == 5)
    ).length"#,
    )
    .expect_number(2.0);
}

/// Verifies where alias.
#[test]
fn test_where_alias() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].where(|x| x > 3)
    ).length"#,
    )
    .expect_number(2.0);
}

/// Verifies find no match returns none.
#[test]
fn test_find_no_match_returns_none() {
    ShapeTest::new(
        r#"
        [1, 2, 3].find(|x| x > 100)
    "#,
    )
    .expect_none();
}

/// Verifies distinct by.
#[test]
fn test_distinct_by() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5, 6].distinctBy(|x| x % 3)
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies lambda float arithmetic.
#[test]
fn test_lambda_float_arithmetic() {
    ShapeTest::new(
        r#"
        let f = |x| x * 2.5
        f(4.0)
    "#,
    )
    .expect_number(10.0);
}

/// Verifies map float values.
#[test]
fn test_map_float_values() {
    ShapeTest::new(
        r#"
        [1.0, 2.0, 3.0].map(|x| x * 0.5)
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies nested map calls.
#[test]
fn test_nested_map_calls() {
    ShapeTest::new(
        r#"(
        [[1, 2], [3, 4]].map(|inner| inner.map(|x| x * 10))
    )[0][0]"#,
    )
    .expect_number(10.0);
}

/// Verifies lambda returning array.
#[test]
fn test_lambda_returning_array() {
    ShapeTest::new(
        r#"
        let pair = |x| [x, x * 2]
        pair(5)
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies single element filter pass.
#[test]
fn test_single_element_filter_pass() {
    ShapeTest::new(
        r#"(
        [42].filter(|x| x > 0)
    ).length"#,
    )
    .expect_number(1.0);
}

/// Verifies single element filter fail.
#[test]
fn test_single_element_filter_fail() {
    ShapeTest::new(
        r#"(
        [42].filter(|x| x < 0)
    ).length"#,
    )
    .expect_number(0.0);
}

/// Verifies single element some true.
#[test]
fn test_single_element_some_true() {
    ShapeTest::new(
        r#"
        [42].some(|x| x == 42)
    "#,
    )
    .expect_bool(true);
}

/// Verifies single element every true.
#[test]
fn test_single_element_every_true() {
    ShapeTest::new(
        r#"
        [42].every(|x| x == 42)
    "#,
    )
    .expect_bool(true);
}

/// Verifies closures in array.
#[test]
fn test_closures_in_array() {
    ShapeTest::new(
        r#"
        let fns = [|x| x + 1, |x| x * 2, |x| x - 3]
        fns[1](10)
    "#,
    )
    .expect_number(20.0);
}

/// Verifies closures in array invoke each.
#[test]
fn test_closures_in_array_invoke_each() {
    ShapeTest::new(
        r#"
        let fns = [|x| x + 1, |x| x * 2, |x| x - 3]
        [fns[0](10), fns[1](10), fns[2](10)]
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies named recursive fn.
#[test]
fn test_named_recursive_fn() {
    ShapeTest::new(
        r#"
        fn factorial(n) {
            if n <= 1 { return 1 }
            return n * factorial(n - 1)
        }
        factorial(5)
    "#,
    )
    .expect_number(120.0);
}

/// Verifies map to arrays.
#[test]
fn test_map_to_arrays() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| [x, x * x])
    ).length"#,
    )
    .expect_number(3.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| [x, x * x])
    )[0][0]"#,
    )
    .expect_number(1.0);
}

/// Verifies lambda on booleans.
#[test]
fn test_lambda_on_booleans() {
    ShapeTest::new(
        r#"
        let negate = |b| !b
        negate(true)
    "#,
    )
    .expect_bool(false);
}

/// Verifies lambda on booleans double.
#[test]
fn test_lambda_on_booleans_double() {
    ShapeTest::new(
        r#"
        let negate = |b| !b
        negate(negate(true))
    "#,
    )
    .expect_bool(true);
}

/// Verifies filter booleans.
#[test]
fn test_filter_booleans() {
    ShapeTest::new(
        r#"(
        [true, false, true, false, true].filter(|x| x)
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies closure captures function param.
#[test]
fn test_closure_captures_function_param() {
    ShapeTest::new(
        r#"
        fn make_checker(threshold) {
            return |x| x > threshold
        }
        let above10 = make_checker(10)
        [above10(5), above10(15)]
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies hof returning hof.
#[test]
fn test_hof_returning_hof() {
    ShapeTest::new(
        r#"
        fn make_mapper(f) {
            return |arr| arr.map(f)
        }
        let double_all = make_mapper(|x| x * 2)
        double_all([1, 2, 3])
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies reduce empty array.
#[test]
fn test_reduce_empty_array() {
    ShapeTest::new(
        r#"
        [].reduce(|acc, x| acc + x, 42)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies map preserves length.
#[test]
fn test_map_preserves_length() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5].map(|x| x).length
    "#,
    )
    .expect_number(5.0);
}
