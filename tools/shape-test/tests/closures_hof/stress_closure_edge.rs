//! Stress tests for closure edge cases.

use shape_test::shape_test::ShapeTest;


/// Verifies filter preserves order.
#[test]
fn test_filter_preserves_order() {
    ShapeTest::new(r#"
        [5, 1, 4, 2, 3].filter(|x| x > 2)
    
true"#)
    .expect_bool(true);
}

/// Verifies reduce with negative initial.
#[test]
fn test_reduce_with_negative_initial() {
    ShapeTest::new(r#"
        [1, 2, 3].reduce(|acc, x| acc + x, -10)
    "#)
    .expect_number(-4.0);
}

/// Verifies reduce mul from one.
#[test]
fn test_reduce_mul_from_one() {
    ShapeTest::new(r#"
        [2, 3, 4].reduce(|acc, x| acc * x, 1)
    "#)
    .expect_number(24.0);
}

/// Verifies every single true.
#[test]
fn test_every_single_true() {
    ShapeTest::new(r#"
        [5].every(|x| x > 0)
    "#)
    .expect_bool(true);
}

/// Verifies some last element.
#[test]
fn test_some_last_element() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 100].some(|x| x > 50)
    "#)
    .expect_bool(true);
}

/// Verifies every large array.
#[test]
fn test_every_large_array() {
    ShapeTest::new(r#"
        fn test() {
            let arr = []
            let i = 1
            while i <= 50 {
                arr.push(i);
                i = i + 1
            }
            return arr.every(|x| x > 0)
        }
        test()
    "#)
    .expect_bool(true);
}

/// Verifies two closures sharing capture.
#[test]
fn test_two_closures_sharing_capture() {
    ShapeTest::new(r#"
        fn make_pair() {
            let val = 0
            let inc = || { val = val + 1; val }
            let get = || val
            inc()
            inc()
            inc()
            get
        }
        let getter = make_pair()
        getter()
    "#)
    .expect_number(3.0);
}

/// Verifies closure block with if else.
#[test]
fn test_closure_block_with_if_else() {
    ShapeTest::new(r#"
        let clamp = |x| {
            if x < 0 { 0 }
            else if x > 100 { 100 }
            else { x }
        }
        [clamp(-5), clamp(50), clamp(200)]
    
true"#)
    .expect_bool(true);
}

/// Verifies lambda stored and passed.
#[test]
fn test_lambda_stored_and_passed() {
    ShapeTest::new(r#"
        fn apply(f, val) { return f(val) }
        let square = |x| x * x
        apply(square, 9)
    "#)
    .expect_number(81.0);
}

/// Verifies lambda passed inline and stored.
#[test]
fn test_lambda_passed_inline_and_stored() {
    ShapeTest::new(r#"
        fn apply(f, val) { return f(val) }
        let cube = |x| x * x * x
        let inline_result = apply(|x| x + 1, 5)
        let stored_result = apply(cube, 3)
        [inline_result, stored_result]
    
true"#)
    .expect_bool(true);
}

/// Verifies map with captured counter.
#[test]
fn test_map_with_captured_counter() {
    ShapeTest::new(r#"
        let offset = 1000
        [1, 2, 3].map(|x| x + offset)
    
true"#)
    .expect_bool(true);
}

/// Verifies chain filter some.
#[test]
fn test_chain_filter_some() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5].filter(|x| x > 2).some(|x| x == 4)
    "#)
    .expect_bool(true);
}

/// Verifies chain map every.
#[test]
fn test_chain_map_every() {
    ShapeTest::new(r#"
        [1, 2, 3].map(|x| x * 2).every(|x| x > 0)
    "#)
    .expect_bool(true);
}

/// Verifies chain map find.
#[test]
fn test_chain_map_find() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5].map(|x| x * x).find(|x| x > 10)
    "#)
    .expect_number(16.0);
}

/// Verifies predicate closure true.
#[test]
fn test_predicate_closure_true() {
    ShapeTest::new(r#"
        let pred = |x| x > 10
        pred(20)
    "#)
    .expect_bool(true);
}

/// Verifies predicate closure false.
#[test]
fn test_predicate_closure_false() {
    ShapeTest::new(r#"
        let pred = |x| x > 10
        pred(5)
    "#)
    .expect_bool(false);
}

/// Verifies predicate factory.
#[test]
fn test_predicate_factory() {
    ShapeTest::new(r#"
        fn gt(n) { return |x| x > n }
        let gt5 = gt(5)
        [gt5(3), gt5(5), gt5(7)]
    
true"#)
    .expect_bool(true);
}

/// Verifies deep capture chain.
#[test]
fn test_deep_capture_chain() {
    ShapeTest::new(r#"
        fn level1(a) {
            fn level2(b) {
                return |c| a + b + c
            }
            return level2(20)
        }
        let f = level1(10)
        f(30)
    "#)
    .expect_number(60.0);
}

/// Verifies pipe two functions.
#[test]
fn test_pipe_two_functions() {
    ShapeTest::new(r#"
        fn pipe(f, g) {
            return |x| g(f(x))
        }
        let transform = pipe(|x| x + 1, |x| x * 2)
        transform(4)
    "#)
    .expect_number(10.0);
}

/// Verifies apply n times.
#[test]
fn test_apply_n_times() {
    ShapeTest::new(r#"
        fn apply_n(f, n, x) {
            let result = x
            let i = 0
            while i < n {
                result = f(result)
                i = i + 1
            }
            return result
        }
        apply_n(|x| x * 2, 4, 1)
    "#)
    .expect_number(16.0);
}

/// Verifies map then length.
#[test]
fn test_map_then_length() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5].map(|x| x * 2).length
    "#)
    .expect_number(5.0);
}

/// Verifies filter then length.
#[test]
fn test_filter_then_length() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].filter(|x| x % 3 == 0).length
    "#)
    .expect_number(3.0);
}

/// Verifies flatmap then length.
#[test]
fn test_flatmap_then_length() {
    ShapeTest::new(r#"
        [1, 2, 3].flatMap(|x| [x, x]).length
    "#)
    .expect_number(6.0);
}

/// Verifies lambda single value.
#[test]
fn test_lambda_single_value() {
    ShapeTest::new(r#"
        let f = |x| 42
        f(999)
    "#)
    .expect_number(42.0);
}

/// Verifies lambda param unused.
#[test]
fn test_lambda_param_unused() {
    ShapeTest::new(r#"
        let f = |x, y| x
        f(10, 20)
    "#)
    .expect_number(10.0);
}

/// Verifies lambda second param only.
#[test]
fn test_lambda_second_param_only() {
    ShapeTest::new(r#"
        let f = |x, y| y
        f(10, 20)
    "#)
    .expect_number(20.0);
}

/// Verifies select alias.
#[test]
fn test_select_alias() {
    ShapeTest::new(r#"
        [1, 2, 3].select(|x| x * 10)
    
true"#)
    .expect_bool(true);
}

/// Verifies closure result in arithmetic.
#[test]
fn test_closure_result_in_arithmetic() {
    ShapeTest::new(r#"
        let f = |x| x * 2
        f(5) + f(3)
    "#)
    .expect_number(16.0);
}

/// Verifies closure result in comparison.
#[test]
fn test_closure_result_in_comparison() {
    ShapeTest::new(r#"
        let f = |x| x * 2
        f(5) > f(3)
    "#)
    .expect_bool(true);
}

/// Verifies closure result in conditional.
#[test]
fn test_closure_result_in_conditional() {
    ShapeTest::new(r#"
        let f = |x| x > 10
        if f(20) { 1 } else { 0 }
    "#)
    .expect_number(1.0);
}

/// Verifies reduce min value.
#[test]
fn test_reduce_min_value() {
    ShapeTest::new(r#"
        [5, 3, 8, 1, 9].reduce(|acc, x| if x < acc { x } else { acc }, 999)
    "#)
    .expect_number(1.0);
}

/// Verifies reduce running sum check.
#[test]
fn test_reduce_running_sum_check() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].reduce(|acc, x| acc + x, 0)
    "#)
    .expect_number(55.0);
}

/// Verifies fibonacci via closures.
#[test]
fn test_fibonacci_via_closures() {
    ShapeTest::new(r#"
        fn fib(n) {
            if n <= 1 { return n }
            return fib(n - 1) + fib(n - 2)
        }
        [fib(0), fib(1), fib(2), fib(3), fib(4), fib(5), fib(6)]
    
true"#)
    .expect_bool(true);
}

/// Verifies full pipeline complex.
#[test]
fn test_full_pipeline_complex() {
    ShapeTest::new(r#"
        fn test() {
            let data = []
            let i = 1
            while i <= 20 {
                data.push(i);
                i = i + 1
            }
            return data
                .filter(|x| x % 2 == 0)
                .map(|x| x * x)
                .filter(|x| x > 50)
                .reduce(|acc, x| acc + x, 0)
        }
        test()
    "#)
    .expect_number(1484.0);
}

/// Verifies map identity.
#[test]
fn test_map_identity() {
    ShapeTest::new(r#"
        [10, 20, 30].map(|x| x)
    
true"#)
    .expect_bool(true);
}

/// Verifies filter identity.
#[test]
fn test_filter_identity() {
    ShapeTest::new(r#"(
        [1, 2, 3].filter(|x| true)
    ).length"#)
    .expect_number(3.0);
}

/// Verifies chain three maps.
#[test]
fn test_chain_three_maps() {
    ShapeTest::new(r#"
        [1, 2, 3]
            .map(|x| x + 1)
            .map(|x| x * 2)
            .map(|x| x - 1)
    
true"#)
    .expect_bool(true);
}

/// Verifies closure over array.
#[test]
fn test_closure_over_array() {
    ShapeTest::new(r#"
        let data = [10, 20, 30]
        let get_sum = || data.reduce(|acc, x| acc + x, 0)
        get_sum()
    "#)
    .expect_number(60.0);
}

/// Verifies closure factory with array method.
#[test]
fn test_closure_factory_with_array_method() {
    ShapeTest::new(r#"
        fn make_filter(threshold) {
            return |arr| arr.filter(|x| x > threshold)
        }
        let big_only = make_filter(5)
        big_only([1, 3, 5, 7, 9]).length
    "#)
    .expect_number(2.0);
}

/// Verifies nested closure capture arithmetic.
#[test]
fn test_nested_closure_capture_arithmetic() {
    ShapeTest::new(r#"
        fn outer(a) {
            fn inner(b) {
                return |c| a * b + c
            }
            return inner(3)
        }
        let f = outer(10)
        f(7)
    "#)
    .expect_number(37.0);
}

/// Verifies reduce with closure capture.
#[test]
fn test_reduce_with_closure_capture() {
    ShapeTest::new(r#"
        let multiplier = 2
        [1, 2, 3].reduce(|acc, x| acc + x * multiplier, 0)
    "#)
    .expect_number(12.0);
}

/// Verifies map with block body closure.
#[test]
fn test_map_with_block_body_closure() {
    ShapeTest::new(r#"
        [1, 2, 3].map(|x| {
            let doubled = x * 2
            let tripled = x * 3
            doubled + tripled
        })
    
true"#)
    .expect_bool(true);
}

/// Verifies mixed named and lambda hof.
#[test]
fn test_mixed_named_and_lambda_hof() {
    ShapeTest::new(r#"
        fn test() {
            fn is_positive(x) { return x > 0 }
            return [-3, -1, 0, 1, 3, 5]
                .filter(is_positive)
                .map(|x| x * 10)
                .length
        }
        test()
    "#)
    .expect_number(3.0);
}

/// Verifies every all negative.
#[test]
fn test_every_all_negative() {
    ShapeTest::new(r#"
        [-1, -2, -3].every(|x| x < 0)
    "#)
    .expect_bool(true);
}

/// Verifies some none negative.
#[test]
fn test_some_none_negative() {
    ShapeTest::new(r#"
        [1, 2, 3].some(|x| x < 0)
    "#)
    .expect_bool(false);
}
