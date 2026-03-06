use shape_test::shape_test::ShapeTest;

// =============================================================================
// Borrow Rules — from programs_borrow_and_refs.rs (test_borrow_*)
// =============================================================================

#[test]
fn test_borrow_two_shared_reads_same_var_ok() {
    // Two shared borrows of same variable should be allowed
    ShapeTest::new(
        r#"
        fn sum_pair(a, b) { a[0] + b[0] }
        let xs = [7]
        sum_pair(xs, xs)
    "#,
    )
    .expect_number(14.0);
}

#[test]
fn test_borrow_exclusive_then_exclusive_same_var_error() {
    // Two exclusive borrows of same variable should be detected at compile time [B0001]
    ShapeTest::new(
        r#"
        fn take2(&a, &b) { a = b }
        fn test() {
            let x = 5
            take2(&x, &x)
        }
    "#,
    )
    .expect_semantic_diagnostic_contains("[B0001]");
}

#[test]
fn test_borrow_shared_plus_exclusive_same_var_error() {
    // Mixed shared+exclusive borrow of same variable detected at compile time [B0001]
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
    "#,
    )
    .expect_semantic_diagnostic_contains("[B0001]");
}

#[test]
fn test_borrow_sequential_exclusive_borrows_ok() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        inc(&a)
        inc(&a)
        inc(&a)
        a
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_borrow_different_vars_independent_ok() {
    ShapeTest::new(
        r#"
        fn swap(&a, &b) {
            let tmp = a
            a = b
            b = tmp
        }
        let x = 10
        let y = 20
        swap(&x, &y)
        x * 100 + y
    "#,
    )
    .expect_number(2010.0);
}

#[test]
fn test_borrow_released_at_scope_exit_reborrow_ok() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        {
            inc(&a)
        }
        {
            inc(&a)
        }
        a
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn test_borrow_nested_scope_borrows() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        let b = 0
        {
            inc(&a)
            {
                inc(&b)
            }
            inc(&a)
        }
        inc(&a)
        inc(&b)
        a * 10 + b
    "#,
    )
    .expect_number(32.0);
}

#[test]
fn test_borrow_in_for_loop_body() {
    ShapeTest::new(
        r#"
        fn add_to(&total, val) { total = total + val }
        let sum = 0
        for item in [10, 20, 30, 40, 50] {
            add_to(&sum, item)
        }
        sum
    "#,
    )
    .expect_number(150.0);
}

#[test]
fn test_borrow_in_while_loop_body() {
    ShapeTest::new(
        r#"
        fn double(&x) { x = x * 2 }
        let val = 1
        let i = 0
        while i < 4 {
            double(&val)
            i = i + 1
        }
        val
    "#,
    )
    .expect_number(16.0);
}

#[test]
fn test_borrow_across_function_calls() {
    // Different function calls can each borrow exclusively if sequential
    ShapeTest::new(
        r#"
        fn read(&x) { x }
        fn inc(&x) { x = x + 1 }
        let a = 10
        let v1 = read(&a)
        inc(&a)
        let v2 = read(&a)
        v1 * 100 + v2
    "#,
    )
    .expect_number(1011.0);
}

#[test]
fn test_borrow_three_shared_borrows_ok() {
    ShapeTest::new(
        r#"
        fn sum3(a, b, c) { a[0] + b[0] + c[0] }
        let xs = [10]
        sum3(xs, xs, xs)
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_borrow_if_then_branch() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        if true {
            inc(&a)
        }
        a
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn test_borrow_if_else_branches() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        fn dec(&x) { x = x - 1 }
        let a = 10
        if false {
            inc(&a)
        } else {
            dec(&a)
        }
        a
    "#,
    )
    .expect_number(9.0);
}

#[test]
fn test_borrow_nested_while_loops() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let total = 0
        let i = 0
        while i < 3 {
            let j = 0
            while j < 3 {
                inc(&total)
                j = j + 1
            }
            i = i + 1
        }
        total
    "#,
    )
    .expect_number(9.0);
}

#[test]
fn test_borrow_assign_after_borrow_release() {
    ShapeTest::new(
        r#"
        fn read(&x) { x }
        let a = 5
        let v = read(&a)
        a = 100
        a + v
    "#,
    )
    .expect_number(105.0);
}

#[test]
fn test_borrow_two_mutating_params_different_vars_ok() {
    ShapeTest::new(
        r#"
        fn swap_first(a, b) {
            let t = a[0]
            a[0] = b[0]
            b[0] = t
        }
        let xs = [1]
        let ys = [2]
        swap_first(xs, ys)
        xs[0] * 10 + ys[0]
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn test_borrow_two_mutating_params_same_var_error() {
    // Two mutating params aliased to same variable detected at compile time [B0001]
    ShapeTest::new(
        r#"
        fn swap_first(a, b) {
            let t = a[0]
            a[0] = b[0]
            b[0] = t
        }
        fn test() {
            let xs = [1]
            swap_first(xs, xs)
        }
    "#,
    )
    .expect_semantic_diagnostic_contains("[B0001]");
}

#[test]
fn test_borrow_deeply_nested_scopes_5_levels() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        {
            inc(&a)
            {
                inc(&a)
                {
                    inc(&a)
                    {
                        inc(&a)
                        {
                            inc(&a)
                        }
                    }
                }
            }
        }
        a
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_borrow_in_conditional_branches_independent() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        let b = 0
        if true {
            inc(&a)
        } else {
            inc(&b)
        }
        inc(&a)
        inc(&b)
        a * 10 + b
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn test_borrow_for_in_with_ref_accumulator() {
    ShapeTest::new(
        r#"
        fn add_to(&acc, val) { acc = acc + val }
        let sum = 0
        for x in [10, 20, 30] {
            add_to(&sum, x)
        }
        sum
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn test_borrow_alternating_vars_in_loop() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        let b = 0
        let i = 0
        while i < 6 {
            if i % 2 == 0 {
                inc(&a)
            } else {
                inc(&b)
            }
            i = i + 1
        }
        a * 10 + b
    "#,
    )
    .expect_number(33.0);
}

#[test]
fn test_borrow_fibonacci_via_refs() {
    ShapeTest::new(
        r#"
        fn fib_step(&a, &b) {
            let tmp = a + b
            a = b
            b = tmp
        }
        let a = 0
        let b = 1
        let i = 0
        while i < 10 {
            fib_step(&a, &b)
            i = i + 1
        }
        b
    "#,
    )
    .expect_number(89.0);
}

#[test]
fn test_borrow_read_and_inc_return_old_value() {
    // Note: `let old = x` in a &x context captures the ref alias,
    // so `old` sees the mutated value. The result is v1=11, v2=12, a=12.
    ShapeTest::new(
        r#"
        fn read_and_inc(&x) {
            let old = x
            x = x + 1
            old
        }
        let a = 10
        let v1 = read_and_inc(&a)
        let v2 = read_and_inc(&a)
        v1 * 100 + v2 * 10 + a
    "#,
    )
    .expect_number(1122.0);
}

#[test]
fn test_borrow_loop_accumulator_sum_1_to_100() {
    ShapeTest::new(
        r#"
        fn add_to(&sum, val) { sum = sum + val }
        let total = 0
        let i = 1
        while i <= 100 {
            add_to(&total, i)
            i = i + 1
        }
        total
    "#,
    )
    .expect_number(5050.0);
}

#[test]
fn test_borrow_scalar_param_no_auto_ref() {
    // Scalar (value) params should not be auto-promoted to ref
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        let x = 5
        add(x, x)
    "#,
    )
    .expect_number(10.0);
}
