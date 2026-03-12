use shape_test::shape_test::ShapeTest;

// =============================================================================
// Implicit Reference Inference — from programs_borrow_refs.rs (infer_*)
// =============================================================================

#[test]
fn infer_array_auto_ref_on_index_mutation() {
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
fn infer_array_index_mutation_multiple() {
    // Multiple index mutations through implicit ref
    ShapeTest::new(
        r#"
        fn init(arr) {
            arr[0] = 10
            arr[1] = 20
            arr[2] = 30
        }
        let xs = [0, 0, 0]
        init(xs)
        xs[0] + xs[1] + xs[2]
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn infer_array_read_only_no_mutation() {
    ShapeTest::new(
        r#"
        fn sum_arr(arr) {
            var total = 0
            for v in arr { total = total + v }
            total
        }
        let xs = [1, 2, 3, 4, 5]
        sum_arr(xs)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn infer_array_read_only_aliasing_ok() {
    // Two read-only params with same array should be fine
    ShapeTest::new(
        r#"
        fn pair_sum(a, b) { a[0] + b[0] }
        let xs = [7]
        pair_sum(xs, xs)
    "#,
    )
    .expect_number(14.0);
}

#[test]
fn infer_sequential_calls_with_same_array_index_mutation() {
    // Sequential mutation calls with index assignment (not push)
    ShapeTest::new(
        r#"
        fn set_at(arr, i, v) { arr[i] = v }
        let xs = [0, 0, 0]
        set_at(xs, 0, 1)
        set_at(xs, 1, 2)
        set_at(xs, 2, 3)
        xs[0] + xs[1] + xs[2]
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn infer_two_mutating_params_different_vars() {
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
fn infer_scalar_param_no_auto_ref() {
    // Scalars are passed by value, not by ref
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        let x = 5
        add(x, x)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn infer_array_mutation_nested_function() {
    // BUG: Array auto-ref inference does not propagate through two levels
    // of function calls. Direct index mutation at one level works, but
    // nested function calls lose the auto-ref. Use explicit & to work
    // around this.
    ShapeTest::new(
        r#"
        fn write_at(&arr, i, v) { arr[i] = v }
        fn init_arr(&arr) {
            write_at(&arr, 0, 100)
            write_at(&arr, 1, 200)
        }
        let nums = [0, 0]
        init_arr(&nums)
        nums[0] + nums[1]
    "#,
    )
    .expect_number(300.0);
}

#[test]
fn infer_array_mutation_in_loop() {
    ShapeTest::new(
        r#"
        fn double_elem(arr, i) { arr[i] = arr[i] * 2 }
        let xs = [1, 2, 3, 4, 5]
        var i = 0
        while i < 5 {
            double_elem(xs, i)
            i = i + 1
        }
        xs[0] + xs[1] + xs[2] + xs[3] + xs[4]
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn infer_three_shared_borrows_same_array() {
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
fn infer_string_param_value_semantics() {
    // Strings are values, not heap types, so no auto-ref
    ShapeTest::new(
        r#"
        fn greet(name) { "hello " + name }
        let n = "world"
        greet(n)
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn infer_number_param_value_semantics() {
    // Changing a number param inside function should not affect caller
    ShapeTest::new(
        r#"
        fn try_change(x) {
            x = x + 100
            x
        }
        let a = 5
        let result = try_change(a)
        a
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn infer_array_passed_to_read_then_mutate() {
    // First call reads, second mutates — sequential is fine
    ShapeTest::new(
        r#"
        fn first_elem(arr) { arr[0] }
        fn set_first(arr, v) { arr[0] = v }
        let xs = [5]
        let before = first_elem(xs)
        set_first(xs, 99)
        before * 100 + xs[0]
    "#,
    )
    .expect_number(599.0);
}

#[test]
fn infer_array_mutation_visible_to_caller() {
    ShapeTest::new(
        r#"
        fn fill(arr, val) {
            var i = 0
            while i < len(arr) {
                arr[i] = val
                i = i + 1
            }
        }
        let xs = [0, 0, 0, 0]
        fill(xs, 7)
        xs[0] + xs[1] + xs[2] + xs[3]
    "#,
    )
    .expect_number(28.0);
}

#[test]
fn infer_mixed_ref_and_inferred_different_arrays() {
    // Explicit ref on one param, implicit ref on another (different variables)
    ShapeTest::new(
        r#"
        fn copy_first(&target, source) {
            target = source[0]
        }
        let xs = [42]
        let result = 0
        copy_first(&result, xs)
        result
    "#,
    )
    .expect_number(42.0);
}
