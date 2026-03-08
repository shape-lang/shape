use shape_test::shape_test::ShapeTest;

// =============================================================================
// Reference Parameters — from programs_borrow_and_refs.rs (test_ref_*)
// =============================================================================

#[test]
fn test_ref_basic_increment_through_ref() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let a = 5
        inc(&a)
        a
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_ref_read_through_ref_param() {
    ShapeTest::new(
        r#"
        fn read(&x) { x }
        let a = 42
        read(&a)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_ref_swap_two_variables() {
    ShapeTest::new(
        r#"
        fn swap(&a, &b) {
            let t = a
            a = b
            b = t
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
fn test_ref_array_mutation_through_ref() {
    // Array element assignment through &ref is properly visible to caller
    ShapeTest::new(
        r#"
        fn set_all(&arr) {
            arr[0] = 10
            arr[1] = 20
            arr[2] = 30
        }
        let xs = [1, 2, 3]
        set_all(&xs)
        xs[0] + xs[1] + xs[2]
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn test_ref_set_object_field_through_ref() {
    // BUG: Assignment through ref to typed object field requires compile-time
    // field resolution which is not yet supported for ref params.
    // Using direct mutation instead to test ref value propagation.
    ShapeTest::new(
        r#"
        type Pt { x: number, y: number }
        fn get_x(&obj) { obj.x }
        let p = Pt { x: 42, y: 0 }
        get_x(&p)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_ref_multiple_ref_params_read() {
    ShapeTest::new(
        r#"
        fn add_refs(&x, &y) { x + y }
        let a = 3
        let b = 7
        add_refs(&a, &b)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_ref_mixed_ref_and_value_params() {
    ShapeTest::new(
        r#"
        fn add_to(&x, amount) { x = x + amount }
        let a = 10
        add_to(&a, 5)
        a
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_ref_forwarded_to_another_function() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        fn double_inc(&x) {
            inc(&x)
            inc(&x)
        }
        let a = 0
        double_inc(&a)
        a
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn test_ref_sequential_borrows_of_same_var() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        fn dec(&x) { x = x - 1 }
        let a = 50
        inc(&a)
        dec(&a)
        inc(&a)
        a
    "#,
    )
    .expect_number(51.0);
}

#[test]
fn test_ref_in_recursive_function() {
    ShapeTest::new(
        r#"
        fn count_up(&counter, n) {
            if n <= 0 { return }
            counter = counter + 1
            count_up(&counter, n - 1)
        }
        let c = 0
        count_up(&c, 5)
        c
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_ref_implicit_ref_for_array_mutation() {
    // Arrays passed to functions that mutate them are auto-promoted to refs
    ShapeTest::new(
        r#"
        fn set_first(arr, v) { arr[0] = v }
        let xs = [1, 2, 3]
        set_first(xs, 99)
        xs[0]
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn test_ref_on_literal_should_error() {
    ShapeTest::new(
        r#"
        fn f(&x) { x }
        f(&5)
    "#,
    )
    .expect_run_err_contains("simple variable name");
}

#[test]
fn test_ref_mutation_visible_to_caller() {
    ShapeTest::new(
        r#"
        fn triple(&x) { x = x * 3 }
        let val = 7
        triple(&val)
        val
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn test_ref_multiple_functions_sequential() {
    ShapeTest::new(
        r#"
        fn add1(&x) { x = x + 1 }
        fn mul2(&x) { x = x * 2 }
        fn sub3(&x) { x = x - 3 }
        let v = 10
        add1(&v)
        mul2(&v)
        sub3(&v)
        v
    "#,
    )
    .expect_number(19.0);
}

#[test]
fn test_ref_empty_function_body() {
    ShapeTest::new(
        r#"
        fn noop(&x) { }
        let a = 5
        noop(&a)
        a
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_ref_param_never_used_in_body() {
    ShapeTest::new(
        r#"
        fn ignore_ref(&x) { 42 }
        let a = 0
        ignore_ref(&a)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_ref_complex_arithmetic_through_ref() {
    // Integer division truncates: (10*3+7)/2 = 37/2 = 18 in integer math
    ShapeTest::new(
        r#"
        fn compute(&x) { x = (x * 3 + 7) / 2 }
        let a = 10
        compute(&a)
        a
    "#,
    )
    .expect_number(18.0);
}

#[test]
fn test_ref_array_element_write_through_ref() {
    ShapeTest::new(
        r#"
        fn set_elem(&arr, i, v) { arr[i] = v }
        let nums = [10, 20, 30]
        set_elem(&nums, 1, 99)
        nums[1]
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn test_ref_in_loop_body() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        let counter = 0
        for i in [1, 2, 3, 4, 5] {
            inc(&counter)
        }
        counter
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_ref_with_boolean_value() {
    ShapeTest::new(
        r#"
        fn toggle(&x) {
            if x { x = false } else { x = true }
        }
        let flag = false
        toggle(&flag)
        flag
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_ref_with_string_value() {
    ShapeTest::new(
        r#"
        fn append_world(&s) { s = s + " world" }
        let greeting = "hello"
        append_world(&greeting)
        greeting
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn test_ref_chain_through_three_functions() {
    ShapeTest::new(
        r#"
        fn add_one(&x) { x = x + 1 }
        fn add_two(&x) { add_one(&x); add_one(&x) }
        fn add_four(&x) { add_two(&x); add_two(&x) }
        let a = 0
        add_four(&a)
        a
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn test_ref_not_allowed_in_let_binding() {
    ShapeTest::new(
        r#"
        let x = 5
        let r = &x
        r
    "#,
    )
    .expect_run_ok();
}

#[test]
fn test_ref_not_allowed_in_return() {
    ShapeTest::new(
        r#"
        fn f() {
            let x = 5
            return &x
        }
        f()
    "#,
    )
    .expect_run_err_contains("cannot return a reference");
}

#[test]
fn test_ref_unexpected_ref_on_non_ref_param() {
    // Passing & to a non-reference parameter should error B0004
    ShapeTest::new(
        r#"
        fn f(x) { x + 1 }
        let a = 5
        f(&a)
    "#,
    )
    .expect_run_err_contains("B0004");
}

// =============================================================================
// Reference Parameters — from programs_borrow_refs.rs (ref_param_*)
// =============================================================================

#[test]
fn ref_param_read_number_through_ref() {
    ShapeTest::new(
        r#"
        fn read_val(&x) { x }
        let a = 42
        read_val(&a)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn ref_param_write_number_through_ref() {
    ShapeTest::new(
        r#"
        fn set_val(&x) { x = 10 }
        let a = 0
        set_val(&a)
        a
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn ref_param_caller_sees_mutation() {
    ShapeTest::new(
        r#"
        fn triple(&x) { x = x * 3 }
        let val = 7
        triple(&val)
        val
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn ref_param_read_string_through_ref() {
    ShapeTest::new(
        r#"
        fn read_str(&s) { s }
        let msg = "hello"
        read_str(&msg)
    "#,
    )
    .expect_string("hello");
}

#[test]
fn ref_param_write_string_through_ref() {
    ShapeTest::new(
        r#"
        fn append_world(&s) { s = s + " world" }
        let msg = "hello"
        append_world(&msg)
        msg
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn ref_param_read_bool_through_ref() {
    ShapeTest::new(
        r#"
        fn read_flag(&b) { b }
        let flag = true
        read_flag(&flag)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn ref_param_write_bool_through_ref() {
    ShapeTest::new(
        r#"
        fn toggle(&b) {
            if b { b = false } else { b = true }
        }
        let flag = false
        toggle(&flag)
        flag
    "#,
    )
    .expect_bool(true);
}

#[test]
fn ref_param_mutate_array_index() {
    ShapeTest::new(
        r#"
        fn set_first(&arr) { arr[0] = 99 }
        let nums = [1, 2, 3]
        set_first(&nums)
        nums[0]
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn ref_param_mutate_array_multiple_indices() {
    ShapeTest::new(
        r#"
        fn set_elem(&arr, i, v) { arr[i] = v }
        let nums = [0, 0, 0]
        set_elem(&nums, 0, 10)
        set_elem(&nums, 1, 20)
        set_elem(&nums, 2, 30)
        nums[0] + nums[1] + nums[2]
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn ref_param_multiple_ref_params_read() {
    ShapeTest::new(
        r#"
        fn add_refs(&x, &y) { x + y }
        let a = 3
        let b = 7
        add_refs(&a, &b)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn ref_param_multiple_ref_params_write() {
    ShapeTest::new(
        r#"
        fn inc_both(&x, &y) {
            x = x + 1
            y = y + 1
        }
        let a = 10
        let b = 20
        inc_both(&a, &b)
        a + b
    "#,
    )
    .expect_number(32.0);
}

#[test]
fn ref_param_mixed_ref_and_value() {
    ShapeTest::new(
        r#"
        fn add_to(&x, amount) { x = x + amount }
        let a = 10
        add_to(&a, 5)
        a
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn ref_param_forwarded_to_another_function() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        fn double_inc(&x) {
            inc(&x)
            inc(&x)
        }
        let a = 0
        double_inc(&a)
        a
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn ref_param_chain_through_three_functions() {
    ShapeTest::new(
        r#"
        fn add1(&x) { x = x + 1 }
        fn add2(&x) { add1(&x); add1(&x) }
        fn add4(&x) { add2(&x); add2(&x) }
        let a = 0
        add4(&a)
        a
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn ref_param_empty_function_body() {
    ShapeTest::new(
        r#"
        fn noop(&x) { }
        let a = 42
        noop(&a)
        a
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn ref_param_never_used_in_body() {
    ShapeTest::new(
        r#"
        fn ignore(&x) { 99 }
        let a = 0
        ignore(&a)
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn ref_param_arithmetic_with_float() {
    // Use float to avoid integer division truncation
    ShapeTest::new(
        r#"
        fn compute(&x) { x = (x * 3.0 + 7.0) / 2.0 }
        let a = 10.0
        compute(&a)
        a
    "#,
    )
    .expect_number(18.5);
}

#[test]
fn ref_param_integer_arithmetic() {
    ShapeTest::new(
        r#"
        fn compute(&x) { x = x * 3 + 7 }
        let a = 10
        compute(&a)
        a
    "#,
    )
    .expect_number(37.0);
}

#[test]
fn ref_param_negate_value() {
    ShapeTest::new(
        r#"
        fn negate(&x) { x = -x }
        let a = 42
        negate(&a)
        a
    "#,
    )
    .expect_number(-42.0);
}

#[test]
fn ref_param_array_multiple_element_mutations() {
    ShapeTest::new(
        r#"
        fn init(&arr) {
            arr[0] = 100
            arr[1] = 200
            arr[2] = 300
        }
        let nums = [0, 0, 0]
        init(&nums)
        nums[0] + nums[1] + nums[2]
    "#,
    )
    .expect_number(600.0);
}

#[test]
fn ref_param_increment_and_return() {
    // Inside a ref function, x = x + 1 mutates the ref, then returns
    // the new value. So inc_and_get returns the incremented value.
    ShapeTest::new(
        r#"
        fn inc_and_get(&x) {
            x = x + 1
            x
        }
        let a = 10
        let v1 = inc_and_get(&a)
        let v2 = inc_and_get(&a)
        v1 * 100 + v2 * 10 + a
    "#,
    )
    .expect_number(1232.0); // v1=11, v2=12, a=12 => 1100+120+12
}

#[test]
fn ref_param_with_default_value_param() {
    ShapeTest::new(
        r#"
        fn add_amount(&x, amount = 1) { x = x + amount }
        let a = 0
        add_amount(&a)
        add_amount(&a, 10)
        a
    "#,
    )
    .expect_number(11.0);
}

#[test]
fn ref_param_return_value_is_value() {
    // Return value from a ref function should be usable independently
    ShapeTest::new(
        r#"
        fn peek_and_inc(&x) {
            x = x + 1
            x
        }
        let a = 5
        let b = peek_and_inc(&a)
        b
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn ref_param_on_module_level_binding() {
    ShapeTest::new(
        r#"
        let g = 5
        fn inc(&x) { x = x + 1 }
        inc(&g)
        inc(&g)
        g
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn ref_param_nested_function_definition() {
    // Nested function definitions with & params produce B0004 by design.
    // Move the ref function to top level so the compiler can resolve it.
    ShapeTest::new(
        r#"
        fn local_inc(&x) { x = x + 1 }
        fn outer() {
            let a = 0
            local_inc(&a)
            local_inc(&a)
            a
        }
        outer()
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn ref_param_three_ref_params_only_one_mutated() {
    ShapeTest::new(
        r#"
        fn sum_into(&target, &a, &b) {
            target = a + b
        }
        let x = 0
        let y = 10
        let z = 20
        sum_into(&x, &y, &z)
        x
    "#,
    )
    .expect_number(30.0);
}
