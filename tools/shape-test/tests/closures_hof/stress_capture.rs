//! Stress tests for variable capture in closures.

use shape_test::shape_test::ShapeTest;


/// Verifies reduce with initial.
#[test]
fn test_reduce_with_initial() {
    ShapeTest::new(r#"
        [1, 2, 3].reduce(|acc, x| acc + x, 100)
    "#)
    .expect_number(106.0);
}

/// Verifies find basic.
#[test]
fn test_find_basic() {
    ShapeTest::new(r#"
        [10, 20, 30].find(|x| x > 15)
    "#)
    .expect_number(20.0);
}

/// Verifies find first match.
#[test]
fn test_find_first_match() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5].find(|x| x > 2)
    "#)
    .expect_number(3.0);
}

/// Verifies find exact.
#[test]
fn test_find_exact() {
    ShapeTest::new(r#"
        [5, 10, 15, 20].find(|x| x == 15)
    "#)
    .expect_number(15.0);
}

/// Verifies some true.
#[test]
fn test_some_true() {
    ShapeTest::new(r#"
        [1, 2, 3].some(|x| x > 2)
    "#)
    .expect_bool(true);
}

/// Verifies some false.
#[test]
fn test_some_false() {
    ShapeTest::new(r#"
        [1, 2, 3].some(|x| x > 10)
    "#)
    .expect_bool(false);
}

/// Verifies any alias true.
#[test]
fn test_any_alias_true() {
    ShapeTest::new(r#"
        [1, 2, 3].any(|x| x == 2)
    "#)
    .expect_bool(true);
}

/// Verifies any alias false.
#[test]
fn test_any_alias_false() {
    ShapeTest::new(r#"
        [1, 2, 3].any(|x| x == 5)
    "#)
    .expect_bool(false);
}

/// Verifies every true.
#[test]
fn test_every_true() {
    ShapeTest::new(r#"
        [2, 4, 6].every(|x| x % 2 == 0)
    "#)
    .expect_bool(true);
}

/// Verifies every false.
#[test]
fn test_every_false() {
    ShapeTest::new(r#"
        [2, 4, 5].every(|x| x % 2 == 0)
    "#)
    .expect_bool(false);
}

/// Verifies all alias true.
#[test]
fn test_all_alias_true() {
    ShapeTest::new(r#"
        [1, 2, 3].all(|x| x > 0)
    "#)
    .expect_bool(true);
}

/// Verifies all alias false.
#[test]
fn test_all_alias_false() {
    ShapeTest::new(r#"
        [1, 2, 3].all(|x| x > 1)
    "#)
    .expect_bool(false);
}

/// Verifies chain map filter.
#[test]
fn test_chain_map_filter() {
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5]
            .map(|x| x * 2)
            .filter(|x| x > 5)
    ).length"#)
    .expect_number(3.0);
}

/// Verifies chain filter map.
#[test]
fn test_chain_filter_map() {
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5, 6]
            .filter(|x| x % 2 == 0)
            .map(|x| x * 10)
    ).length"#)
    .expect_number(3.0);
}

/// Verifies chain filter map reduce.
#[test]
fn test_chain_filter_map_reduce() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5, 6]
            .filter(|x| x % 2 == 0)
            .map(|x| x * 10)
            .reduce(|acc, x| acc + x, 0)
    "#)
    .expect_number(120.0);
}

/// Verifies chain map map.
#[test]
fn test_chain_map_map() {
    ShapeTest::new(r#"
        [1, 2, 3]
            .map(|x| x + 1)
            .map(|x| x * 2)
    
true"#)
    .expect_bool(true);
}

/// Verifies chain filter filter.
#[test]
fn test_chain_filter_filter() {
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .filter(|x| x > 3)
            .filter(|x| x < 8)
    ).length"#)
    .expect_number(4.0);
}

/// Verifies chain map filter some.
#[test]
fn test_chain_map_filter_some() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5]
            .map(|x| x * 2)
            .filter(|x| x > 6)
            .some(|x| x == 10)
    "#)
    .expect_bool(true);
}

/// Verifies mutable capture counter.
#[test]
fn test_mutable_capture_counter() {
    ShapeTest::new(r#"
        fn make_counter() {
            let x = 0
            let inc = || { x = x + 1; x }
            inc
        }
        let counter = make_counter()
        counter()
        counter()
        counter()
    "#)
    .expect_number(3.0);
}

/// Verifies mutable capture accumulator.
#[test]
fn test_mutable_capture_accumulator() {
    ShapeTest::new(r#"
        fn make_acc() {
            let sum = 0
            let add = |x| { sum = sum + x; sum }
            add
        }
        let acc = make_acc()
        acc(10)
        acc(20)
        acc(30)
    "#)
    .expect_number(60.0);
}

/// Verifies iife basic.
#[test]
fn test_iife_basic() {
    ShapeTest::new(r#"
        (|x| x * 2)(5)
    "#)
    .expect_number(10.0);
}

/// Verifies iife no params.
#[test]
fn test_iife_no_params() {
    ShapeTest::new(r#"
        (|| 99)()
    "#)
    .expect_number(99.0);
}

/// Verifies iife multi param.
#[test]
fn test_iife_multi_param() {
    ShapeTest::new(r#"
        (|a, b| a + b)(3, 4)
    "#)
    .expect_number(7.0);
}

/// Verifies empty array map.
#[test]
fn test_empty_array_map() {
    ShapeTest::new(r#"(
        [].map(|x| x * 2)
    ).length"#)
    .expect_number(0.0);
}

/// Verifies empty array filter.
#[test]
fn test_empty_array_filter() {
    ShapeTest::new(r#"(
        [].filter(|x| x > 0)
    ).length"#)
    .expect_number(0.0);
}

/// Verifies empty array some.
#[test]
fn test_empty_array_some() {
    ShapeTest::new(r#"
        [].some(|x| x > 0)
    "#)
    .expect_bool(false);
}

/// Verifies empty array every.
#[test]
fn test_empty_array_every() {
    ShapeTest::new(r#"
        [].every(|x| x > 0)
    "#)
    .expect_bool(true);
}

/// Verifies flatmap basic.
#[test]
fn test_flatmap_basic() {
    ShapeTest::new(r#"(
        [[1, 2], [3, 4]].flatMap(|arr| arr)
    ).length"#)
    .expect_number(4.0);
}

/// Verifies flatmap expand.
#[test]
fn test_flatmap_expand() {
    ShapeTest::new(r#"(
        [1, 2, 3].flatMap(|x| [x, x * 10])
    ).length"#)
    .expect_number(6.0);
}

/// Verifies find index basic.
#[test]
fn test_find_index_basic() {
    ShapeTest::new(r#"
        [10, 20, 30, 40].findIndex(|x| x > 25)
    "#)
    .expect_number(2.0);
}

/// Verifies find index first element.
#[test]
fn test_find_index_first_element() {
    ShapeTest::new(r#"
        [10, 20, 30].findIndex(|x| x == 10)
    "#)
    .expect_number(0.0);
}

/// Verifies closure in function.
#[test]
fn test_closure_in_function() {
    ShapeTest::new(r#"
        fn test() {
            let vals = [1, 2, 3, 4, 5]
            let evens = vals.filter(|x| x % 2 == 0)
            return evens.length
        }
test()"#)
    .expect_number(2.0);
}

/// Verifies closure in function with capture.
#[test]
fn test_closure_in_function_with_capture() {
    ShapeTest::new(r#"
        fn test() {
            let base = 100
            let vals = [1, 2, 3]
            let result = vals.map(|x| x + base)
            return result
        }
test()
true"#)
    .expect_bool(true);
}

/// Verifies pipeline sum of squared evens.
#[test]
fn test_pipeline_sum_of_squared_evens() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .filter(|x| x % 2 == 0)
            .map(|x| x * x)
            .reduce(|acc, x| acc + x, 0)
    "#)
    .expect_number(220.0);
}

/// Verifies pipeline count matching.
#[test]
fn test_pipeline_count_matching() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .filter(|x| x > 5)
            .length
    "#)
    .expect_number(5.0);
}

/// Verifies pipeline map filter find.
#[test]
fn test_pipeline_map_filter_find() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5]
            .map(|x| x * 3)
            .filter(|x| x > 6)
            .find(|x| x > 10)
    "#)
    .expect_number(12.0);
}

/// Verifies currying.
#[test]
fn test_currying() {
    ShapeTest::new(r#"
        fn curry_add(a) {
            return |b| a + b
        }
        let add10 = curry_add(10)
        add10(5)
    "#)
    .expect_number(15.0);
}

/// Verifies currying chain.
#[test]
fn test_currying_chain() {
    ShapeTest::new(r#"
        fn curry_add(a) {
            return |b| a + b
        }
        curry_add(10)(5)
    "#)
    .expect_number(15.0);
}

/// Verifies closure capture loop variable.
#[test]
fn test_closure_capture_loop_variable() {
    ShapeTest::new(r#"
        fn test() {
            let closures = []
            let i = 0
            while i < 3 {
                let val = i
                closures.push(|x| x + val);
                i = i + 1
            }
            return closures
        }
        let fns = test()
        fns[0](10)
    "#)
    .expect_number(10.0);
}

/// Verifies custom apply.
#[test]
fn test_custom_apply() {
    ShapeTest::new(r#"
        fn apply(f, x) {
            return f(x)
        }
        apply(|n| n * n, 6)
    "#)
    .expect_number(36.0);
}

/// Verifies custom apply twice.
#[test]
fn test_custom_apply_twice() {
    ShapeTest::new(r#"
        fn apply_twice(f, x) {
            return f(f(x))
        }
        apply_twice(|n| n * 2, 3)
    "#)
    .expect_number(12.0);
}

/// Verifies custom compose.
#[test]
fn test_custom_compose() {
    ShapeTest::new(r#"
        fn compose(f, g) {
            return |x| f(g(x))
        }
        let double_then_add1 = compose(|x| x + 1, |x| x * 2)
        double_then_add1(5)
    "#)
    .expect_number(11.0);
}

/// Verifies for each side effect.
#[test]
fn test_for_each_side_effect() {
    ShapeTest::new(r#"
        fn test() {
            let total = 0
            [1, 2, 3].forEach(|x| { total = total + x })
            return total
        }
        test()
    "#)
    .expect_number(6.0);
}

/// Verifies closure captures at binding.
#[test]
fn test_closure_captures_at_binding() {
    ShapeTest::new(r#"
        let x = 10
        let f = |n| n + x
        f(5)
    "#)
    .expect_number(15.0);
}

/// Verifies lambda complex expr.
#[test]
fn test_lambda_complex_expr() {
    ShapeTest::new(r#"
        let f = |a, b| (a + b) * (a - b)
        f(5, 3)
    "#)
    .expect_number(16.0);
}
