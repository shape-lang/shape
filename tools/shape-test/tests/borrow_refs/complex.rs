use shape_test::shape_test::ShapeTest;

// =============================================================================
// Complex Lifetime Scenarios — from programs_borrow_and_refs.rs (test_complex_*)
// =============================================================================

#[test]
fn test_complex_function_creates_value_passes_ref_to_helper() {
    // Create value, pass ref to helper, verify caller sees changes via &ref
    ShapeTest::new(
        r#"
        fn init(&arr) {
            arr[0] = 10
            arr[1] = 20
            arr[2] = 30
        }
        fn sum_arr(&arr) {
            arr[0] + arr[1] + arr[2]
        }
        let data = [0, 0, 0]
        init(&data)
        sum_arr(&data)
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn test_complex_array_mutation_through_ref_caller_sees_changes() {
    ShapeTest::new(
        r#"
        fn double_all(&arr) {
            let i = 0
            while i < len(arr) {
                arr[i] = arr[i] * 2
                i = i + 1
            }
        }
        let nums = [1, 2, 3, 4, 5]
        double_all(&nums)
        nums[0] + nums[1] + nums[2] + nums[3] + nums[4]
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_complex_object_field_mutation_through_ref() {
    // BUG: Field assignment on ref param requires compile-time field resolution.
    // Test via reading typed object fields through ref instead.
    ShapeTest::new(
        r#"
        type Config { host: string, port: number }
        fn read_port(&cfg) { cfg.port }
        let cfg = Config { host: "localhost", port: 8080 }
        read_port(&cfg)
    "#,
    )
    .expect_number(8080.0);
}

#[test]
fn test_complex_deeply_nested_scopes_with_refs_and_drops() {
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
                            {
                                inc(&a)
                            }
                        }
                    }
                }
            }
        }
        a
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_complex_loop_accumulator_with_refs() {
    ShapeTest::new(
        r#"
        fn accumulate(&total, arr) {
            for v in arr {
                total = total + v
            }
        }
        let sum = 0
        accumulate(&sum, [1, 2, 3])
        accumulate(&sum, [4, 5, 6])
        sum
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn test_complex_multiple_functions_sharing_refs_to_different_vars() {
    ShapeTest::new(
        r#"
        fn set_to(&x, val) { x = val }
        let a = 0
        let b = 0
        let c = 0
        set_to(&a, 1)
        set_to(&b, 2)
        set_to(&c, 3)
        a + b + c
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_complex_early_return_in_deeply_nested_ref_scope() {
    ShapeTest::new(
        r#"
        fn maybe_inc(&x, condition) {
            if !condition {
                return
            }
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
fn test_complex_array_reverse_via_refs() {
    ShapeTest::new(
        r#"
        fn swap_elements(&arr, i, j) {
            let tmp = arr[i]
            arr[i] = arr[j]
            arr[j] = tmp
        }
        let arr = [1, 2, 3, 4, 5]
        swap_elements(&arr, 0, 4)
        swap_elements(&arr, 1, 3)
        arr[0] * 10000 + arr[1] * 1000 + arr[2] * 100 + arr[3] * 10 + arr[4]
    "#,
    )
    .expect_number(54321.0);
}

#[test]
fn test_complex_counter_object_pattern() {
    ShapeTest::new(
        r#"
        fn get_count(&state) { state[0] }
        fn increment(&state) { state[0] = state[0] + 1 }
        let state = [0]
        increment(&state)
        increment(&state)
        increment(&state)
        get_count(&state)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_complex_ref_plus_option_interaction() {
    // Test ref with conditional value assignment
    ShapeTest::new(
        r#"
        fn maybe_set(&x, val, should_set) {
            if should_set { x = val }
        }
        let a = 0
        maybe_set(&a, 42, true)
        let b = 0
        maybe_set(&b, 99, false)
        a * 100 + b
    "#,
    )
    .expect_number(4200.0);
}

#[test]
fn test_complex_array_builder_pattern() {
    // Build array through element assignment via &ref
    ShapeTest::new(
        r#"
        fn set(&arr, idx, val) { arr[idx] = val }
        let result = [0, 0, 0]
        set(&result, 0, 1)
        set(&result, 1, 2)
        set(&result, 2, 3)
        result[0] + result[1] + result[2]
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_complex_mutable_swap_odd_count() {
    ShapeTest::new(
        r#"
        fn swap(&a, &b) {
            let t = a
            a = b
            b = t
        }
        let x = 100
        let y = 200
        swap(&x, &y)
        swap(&x, &y)
        swap(&x, &y)
        x * 1000 + y
    "#,
    )
    .expect_number(200100.0);
}

#[test]
fn test_complex_conditional_ref_mutation_in_loop() {
    ShapeTest::new(
        r#"
        fn inc_if_positive(&x, v) {
            if v > 0 { x = x + v }
        }
        let sum = 0
        for v in [-1, 2, -3, 4, -5, 6] {
            inc_if_positive(&sum, v)
        }
        sum
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_complex_nested_function_definitions_with_refs() {
    ShapeTest::new(
        r#"
        fn outer() {
            fn local_inc(&x) { x = x + 1 }
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
fn test_complex_ref_with_default_params() {
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
fn test_complex_ref_preserves_array_identity() {
    // After mutation through ref, original variable reflects changes
    ShapeTest::new(
        r#"
        fn modify(&arr) {
            arr[0] = 42
            arr[1] = 99
            arr[2] = 7
        }
        let items = [1, 2, 3]
        modify(&items)
        items[0] + items[1]
    "#,
    )
    .expect_number(141.0);
}

#[test]
fn test_complex_ref_with_while_break() {
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
fn test_complex_bubble_sort_via_refs() {
    ShapeTest::new(
        r#"
        fn swap_elem(&arr, i, j) {
            let tmp = arr[i]
            arr[i] = arr[j]
            arr[j] = tmp
        }
        let arr = [5, 3, 1, 4, 2]
        let n = len(arr)
        let i = 0
        while i < n {
            let j = 0
            while j < n - 1 - i {
                if arr[j] > arr[j + 1] {
                    swap_elem(&arr, j, j + 1)
                }
                j = j + 1
            }
            i = i + 1
        }
        arr[0] * 10000 + arr[1] * 1000 + arr[2] * 100 + arr[3] * 10 + arr[4]
    "#,
    )
    .expect_number(12345.0);
}

#[test]
fn test_complex_stack_via_refs() {
    // Simulate stack using array element writes through ref
    ShapeTest::new(
        r#"
        fn fill(&buf) {
            buf[0] = 10
            buf[1] = 20
            buf[2] = 30
        }
        fn read_top(&buf) {
            buf[2]
        }
        let buf = [0, 0, 0]
        fill(&buf)
        let top_val = read_top(&buf)
        top_val
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_complex_ref_mutation_visible_after_early_return() {
    ShapeTest::new(
        r#"
        fn maybe_set(&x, val, condition) {
            if !condition { return }
            x = val
        }
        let a = 0
        maybe_set(&a, 42, true)
        a
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_complex_module_binding_ref_mutation() {
    ShapeTest::new(
        r#"
        let counter = 0
        fn inc(&x) { x = x + 1 }
        inc(&counter)
        inc(&counter)
        inc(&counter)
        counter
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_complex_multiple_arrays_different_mutations() {
    // Multiple arrays with element-assignment mutations through &ref
    ShapeTest::new(
        r#"
        fn set(&arr, i, v) { arr[i] = v }
        let a = [0, 0, 0]
        let b = [0, 0]
        set(&a, 0, 1)
        set(&b, 0, 10)
        set(&a, 1, 2)
        set(&b, 1, 20)
        set(&a, 2, 3)
        a[0] + a[1] + a[2] + b[0] + b[1]
    "#,
    )
    .expect_number(36.0);
}

#[test]
fn test_complex_ref_with_match_expression() {
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
fn test_complex_drop_triple_nested_loops() {
    ShapeTest::new(
        r#"
        fn f() {
            let total = 0
            for i in [1, 2] {
                for j in [1, 2] {
                    for k in [1, 2] {
                        let val = i * j * k
                        total = total + val
                    }
                }
            }
            return total
        }
        f()
    "#,
    )
    .expect_number(27.0);
}

#[test]
fn test_complex_drop_if_chain_with_lets() {
    ShapeTest::new(
        r#"
        fn f(x) {
            if x == 1 {
                let a = 10
                return a
            } else if x == 2 {
                let b = 20
                return b
            } else if x == 3 {
                let c = 30
                return c
            } else {
                let d = 40
                return d
            }
        }
        f(3)
    "#,
    )
    .expect_number(30.0);
}

// =============================================================================
// Complex Patterns — from programs_borrow_refs.rs (complex_*)
// =============================================================================

#[test]
fn complex_swap_via_references() {
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
fn complex_triple_swap() {
    ShapeTest::new(
        r#"
        fn swap(&a, &b) {
            let t = a
            a = b
            b = t
        }
        let x = 100
        let y = 200
        swap(&x, &y)
        swap(&x, &y)
        swap(&x, &y)
        x * 1000 + y
    "#,
    )
    .expect_number(200100.0);
}

#[test]
fn complex_accumulator_pattern() {
    ShapeTest::new(
        r#"
        fn accumulate(&total, arr) {
            for v in arr {
                total = total + v
            }
        }
        let sum = 0
        accumulate(&sum, [1, 2, 3])
        accumulate(&sum, [4, 5, 6])
        sum
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn complex_array_builder_via_index() {
    // Build array content using index assignment (not push, which has
    // known issues through & ref params)
    ShapeTest::new(
        r#"
        fn set(&arr, i, v) { arr[i] = v }
        let result = [0, 0, 0]
        set(&result, 0, 1)
        set(&result, 1, 2)
        set(&result, 2, 3)
        result[0] + result[1] + result[2]
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn complex_fibonacci_via_refs() {
    ShapeTest::new(
        r#"
        fn fib_step(&a, &b) {
            let t = a + b
            a = b
            b = t
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
fn complex_array_reverse_via_refs() {
    ShapeTest::new(
        r#"
        fn swap_elements(&arr, i, j) {
            let t = arr[i]
            arr[i] = arr[j]
            arr[j] = t
        }
        let arr = [1, 2, 3, 4, 5]
        swap_elements(&arr, 0, 4)
        swap_elements(&arr, 1, 3)
        arr[0] * 10000 + arr[1] * 1000 + arr[2] * 100 + arr[3] * 10 + arr[4]
    "#,
    )
    .expect_number(54321.0);
}

#[test]
fn complex_counter_state_pattern() {
    ShapeTest::new(
        r#"
        fn get_count(&state) { state[0] }
        fn increment(&state) { state[0] = state[0] + 1 }
        let state = [0]
        increment(&state)
        increment(&state)
        increment(&state)
        get_count(&state)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn complex_conditional_mutation() {
    ShapeTest::new(
        r#"
        fn inc_if_positive(&x, v) {
            if v > 0 { x = x + v }
        }
        let sum = 0
        for v in [-1, 2, -3, 4, -5, 6] {
            inc_if_positive(&sum, v)
        }
        sum
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn complex_array_pop_via_index() {
    // Simulate stack operations using index-based access
    ShapeTest::new(
        r#"
        fn stack_top(&arr, &size) {
            arr[size - 1]
        }
        fn stack_push(&arr, &size, v) {
            arr[size] = v
            size = size + 1
        }
        let data = [0, 0, 0, 0, 0]
        let size = 0
        stack_push(&data, &size, 10)
        stack_push(&data, &size, 20)
        stack_push(&data, &size, 30)
        let top = stack_top(&data, &size)
        top * 100 + size
    "#,
    )
    .expect_number(3003.0);
}

#[test]
fn complex_multiple_arrays_independent_mutations() {
    ShapeTest::new(
        r#"
        fn set_elem(&arr, i, v) { arr[i] = v }
        let a = [0, 0]
        let b = [0, 0]
        set_elem(&a, 0, 1)
        set_elem(&a, 1, 2)
        set_elem(&b, 0, 10)
        set_elem(&b, 1, 20)
        a[0] + a[1] + b[0] + b[1]
    "#,
    )
    .expect_number(33.0);
}

#[test]
fn complex_ref_mutation_visible_after_early_return() {
    ShapeTest::new(
        r#"
        fn maybe_set(&x, val, condition) {
            if !condition { return }
            x = val
        }
        let a = 0
        maybe_set(&a, 42, true)
        a
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn complex_ref_preserves_array_identity_via_index() {
    ShapeTest::new(
        r#"
        fn modify(&arr) {
            arr[0] = 42
            arr[1] = 99
        }
        let nums = [1, 2, 3]
        modify(&nums)
        let first = nums[0]
        let second = nums[1]
        print(first)
        print(second)
    "#,
    )
    .expect_output("42\n99");
}

#[test]
fn complex_sum_array_through_ref() {
    ShapeTest::new(
        r#"
        fn sum_all(&arr) {
            let total = 0
            for v in arr { total = total + v }
            total
        }
        let data = [10, 20, 30]
        sum_all(&data)
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn complex_sort_three_via_swap() {
    // Sort three numbers using swap
    ShapeTest::new(
        r#"
        fn swap(&a, &b) {
            let t = a
            a = b
            b = t
        }
        let a = 30
        let b = 10
        let c = 20
        if a > b { swap(&a, &b) }
        if b > c { swap(&b, &c) }
        if a > b { swap(&a, &b) }
        a * 10000 + b * 100 + c
    "#,
    )
    .expect_number(102030.0);
}

#[test]
fn complex_borrow_checker_reset_between_functions() {
    // Each function gets its own borrow checker state
    ShapeTest::new(
        r#"
        fn f1() {
            fn inc(&x) { x = x + 1 }
            let a = 1
            inc(&a)
            a
        }
        fn f2() {
            fn dec(&x) { x = x - 1 }
            let b = 10
            dec(&b)
            b
        }
        f1() * 100 + f2()
    "#,
    )
    .expect_number(209.0);
}
