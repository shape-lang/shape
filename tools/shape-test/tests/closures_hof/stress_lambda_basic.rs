//! Stress tests for basic lambda/closure syntax.

use shape_test::shape_test::ShapeTest;

/// Verifies lambda identity.
#[test]
fn test_lambda_identity() {
    ShapeTest::new(
        r#"
        let id = |x| x
        id(42)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies lambda add one.
#[test]
fn test_lambda_add_one() {
    ShapeTest::new(
        r#"
        let f = |x| x + 1
        f(9)
    "#,
    )
    .expect_number(10.0);
}

/// Verifies no param lambda.
#[test]
fn test_no_param_lambda() {
    ShapeTest::new(
        r#"
        let f = || 42
        f()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies no param lambda block.
#[test]
fn test_no_param_lambda_block() {
    ShapeTest::new(
        r#"
        let f = || { let x = 10; x + 5 }
        f()
    "#,
    )
    .expect_number(15.0);
}

/// Verifies lambda multiply.
#[test]
fn test_lambda_multiply() {
    ShapeTest::new(
        r#"
        let f = |x| x * 3
        f(7)
    "#,
    )
    .expect_number(21.0);
}

/// Verifies lambda negate.
#[test]
fn test_lambda_negate() {
    ShapeTest::new(
        r#"
        let neg = |x| -x
        neg(5)
    "#,
    )
    .expect_number(-5.0);
}

/// Verifies lambda boolean return.
#[test]
fn test_lambda_boolean_return() {
    ShapeTest::new(
        r#"
        let is_positive = |x| x > 0
        is_positive(3)
    "#,
    )
    .expect_bool(true);
}

/// Verifies lambda boolean return false.
#[test]
fn test_lambda_boolean_return_false() {
    ShapeTest::new(
        r#"
        let is_positive = |x| x > 0
        is_positive(-1)
    "#,
    )
    .expect_bool(false);
}

/// Verifies lambda two params add.
#[test]
fn test_lambda_two_params_add() {
    ShapeTest::new(
        r#"
        let add = |a, b| a + b
        add(3, 4)
    "#,
    )
    .expect_number(7.0);
}

/// Verifies lambda two params sub.
#[test]
fn test_lambda_two_params_sub() {
    ShapeTest::new(
        r#"
        let sub = |a, b| a - b
        sub(10, 3)
    "#,
    )
    .expect_number(7.0);
}

/// Verifies lambda three params.
#[test]
fn test_lambda_three_params() {
    ShapeTest::new(
        r#"
        let f = |a, b, c| a + b + c
        f(1, 2, 3)
    "#,
    )
    .expect_number(6.0);
}

/// Verifies lambda four params.
#[test]
fn test_lambda_four_params() {
    ShapeTest::new(
        r#"
        let f = |a, b, c, d| a * b + c * d
        f(2, 3, 4, 5)
    "#,
    )
    .expect_number(26.0);
}

/// Verifies lambda block body.
#[test]
fn test_lambda_block_body() {
    ShapeTest::new(
        r#"
        let f = |x| {
            let y = x * 2
            y + 1
        }
        f(5)
    "#,
    )
    .expect_number(11.0);
}

/// Verifies lambda block multiple locals.
#[test]
fn test_lambda_block_multiple_locals() {
    ShapeTest::new(
        r#"
        let f = |x| {
            let a = x + 1
            let b = x * 2
            a + b
        }
        f(3)
    "#,
    )
    .expect_number(10.0);
}

/// Verifies lambda block conditional.
#[test]
fn test_lambda_block_conditional() {
    ShapeTest::new(
        r#"
        let abs = |x| {
            if x < 0 { -x } else { x }
        }
        abs(-7)
    "#,
    )
    .expect_number(7.0);
}

/// Verifies lambda block conditional positive.
#[test]
fn test_lambda_block_conditional_positive() {
    ShapeTest::new(
        r#"
        let abs = |x| {
            if x < 0 { -x } else { x }
        }
        abs(7)
    "#,
    )
    .expect_number(7.0);
}

/// Verifies closure capture one.
#[test]
fn test_closure_capture_one() {
    ShapeTest::new(
        r#"
        let offset = 10
        let f = |x| x + offset
        f(5)
    "#,
    )
    .expect_number(15.0);
}

/// Verifies closure capture string.
#[test]
fn test_closure_capture_string() {
    ShapeTest::new(
        r#"
        let prefix = "hello"
        let f = |x| prefix
        f(0)
    "#,
    )
    .expect_string("hello");
}

/// Verifies closure capture two.
#[test]
fn test_closure_capture_two() {
    ShapeTest::new(
        r#"
        let a = 10
        let b = 20
        let f = |x| x + a + b
        f(5)
    "#,
    )
    .expect_number(35.0);
}

/// Verifies closure capture three.
#[test]
fn test_closure_capture_three() {
    ShapeTest::new(
        r#"
        let a = 1
        let b = 2
        let c = 3
        let f = |x| x + a + b + c
        f(4)
    "#,
    )
    .expect_number(10.0);
}

/// Verifies closure capture arithmetic.
#[test]
fn test_closure_capture_arithmetic() {
    ShapeTest::new(
        r#"
        let scale = 3
        let offset = 7
        let transform = |x| x * scale + offset
        transform(5)
    "#,
    )
    .expect_number(22.0);
}

/// Verifies nested closure basic.
#[test]
fn test_nested_closure_basic() {
    ShapeTest::new(
        r#"
        let outer = |x| {
            let inner = |y| x + y
            inner(10)
        }
        outer(5)
    "#,
    )
    .expect_number(15.0);
}

/// Verifies nested closure chain.
#[test]
fn test_nested_closure_chain() {
    ShapeTest::new(
        r#"
        let a = 1
        let f = |x| {
            let g = |y| x + y + a
            g(10)
        }
        f(100)
    "#,
    )
    .expect_number(111.0);
}

/// Verifies double nested closure.
#[test]
fn test_double_nested_closure() {
    ShapeTest::new(
        r#"
        let f = |x| {
            let g = |y| {
                let h = |z| x + y + z
                h(3)
            }
            g(2)
        }
        f(1)
    "#,
    )
    .expect_number(6.0);
}

/// Verifies closure as arg.
#[test]
fn test_closure_as_arg() {
    ShapeTest::new(
        r#"
        fn apply(f, x) {
            return f(x)
        }
        apply(|x| x * 3, 5)
    "#,
    )
    .expect_number(15.0);
}

/// Verifies closure as arg with capture.
#[test]
fn test_closure_as_arg_with_capture() {
    ShapeTest::new(
        r#"
        fn apply(f, x) {
            return f(x)
        }
        let factor = 4
        apply(|x| x * factor, 5)
    "#,
    )
    .expect_number(20.0);
}

/// Verifies multiple closure args.
#[test]
fn test_multiple_closure_args() {
    ShapeTest::new(
        r#"
        fn compose(f, g, x) {
            return f(g(x))
        }
        compose(|x| x + 1, |x| x * 2, 5)
    "#,
    )
    .expect_number(11.0);
}

/// Verifies closure as return value.
#[test]
fn test_closure_as_return_value() {
    ShapeTest::new(
        r#"
        fn make_adder(n) {
            return |x| x + n
        }
        let add5 = make_adder(5)
        add5(10)
    "#,
    )
    .expect_number(15.0);
}

/// Verifies closure as return value multiplier.
#[test]
fn test_closure_as_return_value_multiplier() {
    ShapeTest::new(
        r#"
        fn make_multiplier(n) {
            return |x| x * n
        }
        let triple = make_multiplier(3)
        triple(7)
    "#,
    )
    .expect_number(21.0);
}

/// Verifies closure factory two instances.
#[test]
fn test_closure_factory_two_instances() {
    ShapeTest::new(
        r#"
        fn make_adder(n) {
            return |x| x + n
        }
        let add3 = make_adder(3)
        let add7 = make_adder(7)
        add3(10) + add7(10)
    "#,
    )
    .expect_number(30.0);
}

/// Verifies map double.
#[test]
fn test_map_double() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| x * 2)
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies map add constant.
#[test]
fn test_map_add_constant() {
    ShapeTest::new(
        r#"
        [10, 20, 30].map(|x| x + 5)
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies map negate.
#[test]
fn test_map_negate() {
    ShapeTest::new(
        r#"
        [1, -2, 3].map(|x| -x)
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies map with capture.
#[test]
fn test_map_with_capture() {
    ShapeTest::new(
        r#"
        let factor = 10
        [1, 2, 3].map(|x| x * factor)
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies map single element.
#[test]
fn test_map_single_element() {
    ShapeTest::new(
        r#"(
        [99].map(|x| x + 1)
    ).length"#,
    )
    .expect_number(1.0);
}

/// Verifies map to boolean.
#[test]
fn test_map_to_boolean() {
    ShapeTest::new(
        r#"
        [1, 0, -1, 2].map(|x| x > 0)
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies filter greater than.
#[test]
fn test_filter_greater_than() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].filter(|x| x > 3)
    ).length"#,
    )
    .expect_number(2.0);
}

/// Verifies filter even.
#[test]
fn test_filter_even() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5, 6].filter(|x| x % 2 == 0)
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies filter none match.
#[test]
fn test_filter_none_match() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].filter(|x| x > 100)
    ).length"#,
    )
    .expect_number(0.0);
}

/// Verifies filter all match.
#[test]
fn test_filter_all_match() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].filter(|x| x > 0)
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies filter with capture.
#[test]
fn test_filter_with_capture() {
    ShapeTest::new(
        r#"
        fn test() {
            let threshold = 3
            return [1, 2, 3, 4, 5].filter(|x| x > threshold).length
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Verifies reduce sum.
#[test]
fn test_reduce_sum() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4].reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(10.0);
}

/// Verifies reduce product.
#[test]
fn test_reduce_product() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4].reduce(|acc, x| acc * x, 1)
    "#,
    )
    .expect_number(24.0);
}

/// Verifies reduce max.
#[test]
fn test_reduce_max() {
    ShapeTest::new(
        r#"
        [3, 1, 4, 1, 5, 9].reduce(|acc, x| if x > acc { x } else { acc }, 0)
    "#,
    )
    .expect_number(9.0);
}

/// Verifies reduce single element.
#[test]
fn test_reduce_single_element() {
    ShapeTest::new(
        r#"
        [42].reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(42.0);
}
