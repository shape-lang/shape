use shape_test::shape_test::ShapeTest;

// =============================================================================
// Borrow Scoping Rules — from programs_borrow_refs.rs (borrow_*)
// =============================================================================

#[test]
fn borrow_sequential_borrows_same_var_ok() {
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
fn borrow_read_original_after_borrow_released() {
    ShapeTest::new(
        r#"
        fn read_val(&x) { x }
        let a = 42
        let b = read_val(&a)
        a
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn borrow_reassign_after_borrow_released() {
    ShapeTest::new(
        r#"
        fn read_val(&x) { x }
        var a = 5
        let v = read_val(&a)
        a = 100
        a + v
    "#,
    )
    .expect_number(105.0);
}

#[test]
fn borrow_in_block_scope_released_at_end() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        {
            inc(&a)
        }
        inc(&a)
        a
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn borrow_in_if_then_branch() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        if true { inc(&a) }
        a
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn borrow_in_if_else_branches() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        fn dec(&x) { x = x - 1 }
        let a = 10
        if false { inc(&a) } else { dec(&a) }
        a
    "#,
    )
    .expect_number(9.0);
}

#[test]
fn borrow_in_while_loop_per_iteration() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let counter = 0
        let i = 0
        while i < 5 {
            inc(&counter)
            i = i + 1
        }
        counter
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn borrow_in_for_loop_accumulator() {
    ShapeTest::new(
        r#"
        fn add_to(&total, val) { total = total + val }
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
fn borrow_nested_blocks_inner_release_before_outer() {
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
fn borrow_different_vars_in_different_scopes() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        let b = 0
        { inc(&a) }
        { inc(&b) }
        a * 10 + b
    "#,
    )
    .expect_number(11.0);
}

#[test]
fn borrow_reborrow_after_scope_exit() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        { inc(&a) }
        { inc(&a) }
        { inc(&a) }
        a
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn borrow_deeply_nested_5_levels() {
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
fn borrow_nested_while_loops() {
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
fn borrow_conditional_branches_independent() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        let b = 0
        if true { inc(&a) } else { inc(&b) }
        inc(&a)
        inc(&b)
        a * 10 + b
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn borrow_for_loop_sum_1_to_100() {
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
fn borrow_different_vars_exclusive_simultaneously() {
    // Two different variables can both be exclusively borrowed at once
    ShapeTest::new(
        r#"
        fn swap(&a, &b) {
            let t = a
            a = b
            b = t
        }
        let x = 42
        let y = 99
        swap(&x, &y)
        x * 1000 + y
    "#,
    )
    .expect_number(99042.0);
}

#[test]
fn borrow_sequential_exclusive_calls_different_fns() {
    ShapeTest::new(
        r#"
        fn double(&x) { x = x * 2 }
        fn add_ten(&x) { x = x + 10 }
        var a = 5
        double(&a)
        add_ten(&a)
        double(&a)
        a
    "#,
    )
    .expect_number(40.0);
}

#[test]
fn borrow_while_loop_doubling() {
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
fn borrow_alternating_vars_in_loop() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        let b = 0
        let i = 0
        while i < 6 {
            if i % 2 == 0 { inc(&a) } else { inc(&b) }
            i = i + 1
        }
        a * 10 + b
    "#,
    )
    .expect_number(33.0);
}

#[test]
fn borrow_while_break_with_ref() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let count = 0
        let i = 0
        while true {
            if i >= 5 { break }
            inc(&count)
            i = i + 1
        }
        count
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn borrow_in_match_expression() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        let val = 2
        match val {
            1 => inc(&a),
            2 => { inc(&a); inc(&a) },
            _ => {}
        }
        a
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn borrow_early_return_in_ref_function() {
    ShapeTest::new(
        r#"
        fn maybe_inc(&x, condition) {
            if !condition { return }
            x = x + 1
        }
        let a = 10
        maybe_inc(&a, true)
        maybe_inc(&a, false)
        maybe_inc(&a, true)
        a
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn borrow_100_sequential_borrows() {
    // Stress test: 100 sequential borrows of same variable
    let mut code = String::from("fn inc(&x) { x = x + 1 }\nlet a = 0\n");
    for _ in 0..100 {
        code.push_str("inc(&a)\n");
    }
    code.push_str("a\n");
    ShapeTest::new(&code).expect_number(100.0);
}

#[test]
fn borrow_many_different_variables() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 0
        let b = 10
        let c = 20
        let d = 30
        let e = 40
        inc(&a)
        inc(&b)
        inc(&c)
        inc(&d)
        inc(&e)
        a + b + c + d + e
    "#,
    )
    .expect_number(105.0);
}

#[test]
fn borrow_recursive_function_with_ref() {
    ShapeTest::new(
        r#"
        fn count_down(&counter, n) {
            if n <= 0 { return }
            counter = counter + 1
            count_down(&counter, n - 1)
        }
        let c = 0
        count_down(&c, 5)
        c
    "#,
    )
    .expect_number(5.0);
}
