use shape_test::shape_test::ShapeTest;

// =============================================================================
// Borrow Violations — from programs_borrow_refs.rs (violation_*)
// =============================================================================

#[test]
fn violation_ref_on_literal_number() {
    ShapeTest::new(
        r#"
        fn f(&x) { x }
        f(&5)
    "#,
    )
    .expect_run_err_contains("simple variable");
}

#[test]
fn violation_ref_on_expression() {
    ShapeTest::new(
        r#"
        fn f(&x) { x = 0 }
        let arr = [1, 2, 3]
        f(&arr[0])
    "#,
    )
    .expect_run_err_contains("simple variable");
}

#[test]
fn violation_ref_in_let_binding() {
    ShapeTest::new(
        r#"
        let x = 5
        let r = &x
    "#,
    )
    .expect_run_err_contains("function arguments");
}

#[test]
fn violation_ref_in_return() {
    ShapeTest::new(
        r#"
        fn f() {
            let x = 5
            return &x
        }
        f()
    "#,
    )
    .expect_run_err_contains("function arguments");
}

#[test]
fn violation_ref_in_array_literal() {
    ShapeTest::new(
        r#"
        let x = 5
        [&x]
    "#,
    )
    .expect_run_err_contains("function arguments");
}

#[test]
fn violation_unexpected_ref_on_non_ref_param() {
    ShapeTest::new(
        r#"
        fn f(x) { x + 1 }
        let a = 5
        f(&a)
    "#,
    )
    .expect_run_err_contains("B0004");
}

#[test]
fn violation_ref_on_string_literal() {
    ShapeTest::new(
        r#"
        fn f(&x) { x }
        f(&"hello")
    "#,
    )
    .expect_run_err_contains("simple variable");
}

#[test]
fn violation_ref_on_boolean_literal() {
    ShapeTest::new(
        r#"
        fn f(&x) { x }
        f(&true)
    "#,
    )
    .expect_run_err_contains("simple variable");
}

#[test]
fn violation_ref_on_array_literal() {
    ShapeTest::new(
        r#"
        fn f(&x) { x[0] = 99 }
        f(&[1, 2, 3])
    "#,
    )
    .expect_run_err_contains("simple variable");
}

#[test]
fn violation_ref_on_function_call_result() {
    ShapeTest::new(
        r#"
        fn make() { 5 }
        fn f(&x) { x }
        f(&make())
    "#,
    )
    .expect_run_err_contains("simple variable");
}

#[test]
fn violation_ref_on_binary_expression() {
    ShapeTest::new(
        r#"
        fn f(&x) { x }
        let a = 1
        let b = 2
        f(&(a + b))
    "#,
    )
    .expect_run_err_contains("simple variable");
}

#[test]
fn violation_ref_in_nested_expression() {
    ShapeTest::new(
        r#"
        fn f(&x) { x }
        let a = 5
        let b = f(&a) + &a
    "#,
    )
    .expect_run_err_contains("function arguments");
}

#[test]
fn violation_ref_as_if_condition() {
    ShapeTest::new(
        r#"
        let x = true
        if &x { 1 } else { 0 }
    "#,
    )
    .expect_run_err_contains("function arguments");
}

#[test]
fn violation_double_exclusive_borrow_in_function() {
    // BUG: Double exclusive borrow of same var may not be caught at top-level.
    // Wrapping in a function to ensure compile-time borrow check runs.
    ShapeTest::new(
        r#"
        fn take2(&a, &b) { a = b }
        fn test() {
            let x = 5
            take2(&x, &x)
        }
        test()
    "#,
    )
    .expect_run_err_contains("B0001");
}

#[test]
fn violation_three_exclusive_refs_same_var_in_function() {
    ShapeTest::new(
        r#"
        fn take3(&a, &b, &c) { a = b + c }
        fn test() {
            let x = 5
            take3(&x, &x, &x)
        }
        test()
    "#,
    )
    .expect_run_err_contains("B0001");
}

#[test]
fn violation_swap_same_var_in_function() {
    ShapeTest::new(
        r#"
        fn swap_vals(&a, &b) {
            let t = a
            a = b
            b = t
        }
        fn test() {
            let x = 1
            swap_vals(&x, &x)
        }
        test()
    "#,
    )
    .expect_run_err_contains("B0001");
}

#[test]
fn violation_mixed_inferred_mutation_aliasing_in_function() {
    // Two params that both mutate the same array (via index assign)
    ShapeTest::new(
        r#"
        fn touch(a, b) {
            a[0] = 1
            b[0]
        }
        fn test() {
            let xs = [5, 9]
            touch(xs, xs)
        }
        test()
    "#,
    )
    .expect_run_err_contains("B0001");
}

#[test]
fn violation_two_mutating_inferred_params_same_var() {
    ShapeTest::new(
        r#"
        fn mutate_both(a, b) {
            a[0] = 99
            b[0] = 88
        }
        fn test() {
            let xs = [1]
            mutate_both(xs, xs)
        }
        test()
    "#,
    )
    .expect_run_err_contains("B0001");
}

#[test]
fn violation_explicit_ref_and_inferred_ref_same_var() {
    ShapeTest::new(
        r#"
        fn modify(&a, b) {
            a = b[0]
        }
        fn test() {
            let xs = [10]
            modify(&xs, xs)
        }
        test()
    "#,
    )
    .expect_run_err_contains("B0001");
}

#[test]
fn violation_ref_on_const_produces_error() {
    // Passing a const variable by exclusive ref correctly errors at compile time.
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        const c = 5
        inc(&c)
    "#,
    )
    .expect_run_err_contains("const");
}
