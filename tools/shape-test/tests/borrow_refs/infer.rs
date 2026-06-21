use shape_test::shape_test::ShapeTest;

// =============================================================================
// Heap-param call convention — CallArgConsume model (user 2026-06-21).
//
// The WS-7 implicit-auto-ref / mutation-share-by-value convention is REVERSED:
// a by-value (non-`&`) HEAP param CONSUMES its arg. Caller-VISIBLE mutation
// requires an explicit `&mut p` param (the loan-back path). Read-only by-value
// heap params still BORROW their receiver via method/index reads, and scalars
// stay Copy. These tests assert the post-reversal behavior — the mutating
// cases use `&mut` (and `&mut`-pass call sites).
// =============================================================================

#[test]
fn mut_ref_array_index_mutation_visible() {
    ShapeTest::new(
        r#"
        fn set_first(arr: &mut Array<int>, v: int) { arr[0] = v }
        let mut xs = [1, 2, 3]
        set_first(&mut xs, 99)
        xs[0]
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn mut_ref_array_index_mutation_multiple() {
    // Multiple index mutations through an explicit `&mut` param.
    ShapeTest::new(
        r#"
        fn init(arr: &mut Array<int>) {
            arr[0] = 10
            arr[1] = 20
            arr[2] = 30
        }
        let mut xs = [0, 0, 0]
        init(&mut xs)
        xs[0] + xs[1] + xs[2]
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn read_only_by_value_param_borrows_receiver() {
    // A read-only by-value heap param BORROWS via the `for v in arr` iteration
    // read — no mutation, the caller's `xs` is consumed by the call but the
    // body only reads. (The binding is not reused after the call here.)
    ShapeTest::new(
        r#"
        fn sum_arr(arr: Array<int>) {
            let mut total = 0
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
fn read_only_clone_aliasing_ok() {
    // Two read-only params that need the SAME array: clone for the second arg
    // so both owners are independent (a by-value heap arg moves).
    ShapeTest::new(
        r#"
        fn pair_sum(a: Array<int>, b: Array<int>) { a[0] + b[0] }
        let xs = [7]
        let ys = clone xs
        pair_sum(xs, ys)
    "#,
    )
    .expect_number(14.0);
}

#[test]
fn mut_ref_sequential_calls_index_mutation() {
    // Sequential mutation calls with index assignment through `&mut`.
    ShapeTest::new(
        r#"
        fn set_at(arr: &mut Array<int>, i: int, v: int) { arr[i] = v }
        let mut xs = [0, 0, 0]
        set_at(&mut xs, 0, 1)
        set_at(&mut xs, 1, 2)
        set_at(&mut xs, 2, 3)
        xs[0] + xs[1] + xs[2]
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn mut_ref_two_mutating_params_different_vars() {
    ShapeTest::new(
        r#"
        fn swap_first(a: &mut Array<int>, b: &mut Array<int>) {
            let t = a[0]
            a[0] = b[0]
            b[0] = t
        }
        let mut xs = [1]
        let mut ys = [2]
        swap_first(&mut xs, &mut ys)
        xs[0] * 10 + ys[0]
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn scalar_param_stays_copy() {
    // Scalars are Copy — passing `x` twice is fine.
    ShapeTest::new(
        r#"
        fn add(a: int, b: int) { a + b }
        let x = 5
        add(x, x)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn mut_ref_array_mutation_nested_function() {
    // Mutation threaded through two `&mut` levels.
    ShapeTest::new(
        r#"
        fn write_at(&arr, i, v) { arr[i] = v }
        fn init_arr(&arr) {
            write_at(&arr, 0, 100)
            write_at(&arr, 1, 200)
        }
        let mut nums = [0, 0]
        init_arr(&nums)
        nums[0] + nums[1]
    "#,
    )
    .expect_number(300.0);
}

#[test]
fn mut_ref_array_mutation_in_loop() {
    ShapeTest::new(
        r#"
        fn double_elem(arr: &mut Array<int>, i: int) { arr[i] = arr[i] * 2 }
        let mut xs = [1, 2, 3, 4, 5]
        let mut i = 0
        while i < 5 {
            double_elem(&mut xs, i)
            i = i + 1
        }
        xs[0] + xs[1] + xs[2] + xs[3] + xs[4]
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn read_only_three_clones_same_value() {
    // Three read-only params needing the same value: clone the extra owners.
    ShapeTest::new(
        r#"
        fn sum3(a: Array<int>, b: Array<int>, c: Array<int>) { a[0] + b[0] + c[0] }
        let xs = [10]
        let ys = clone xs
        let zs = clone xs
        sum3(xs, ys, zs)
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn string_param_value_semantics() {
    // A string arg is consumed; the body reads it. Not reused after the call.
    ShapeTest::new(
        r#"
        fn greet(name: string) { "hello " + name }
        let n = "world"
        greet(n)
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn number_param_stays_copy_no_caller_effect() {
    // Changing a number param inside the function does not affect the caller.
    ShapeTest::new(
        r#"
        fn try_change(x: int) {
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
fn mut_ref_array_read_then_mutate() {
    // First call reads via a borrowing method; second call mutates via `&mut`.
    ShapeTest::new(
        r#"
        fn first_elem(arr: &mut Array<int>) { arr[0] }
        fn set_first(arr: &mut Array<int>, v: int) { arr[0] = v }
        let mut xs = [5]
        let before = first_elem(&mut xs)
        set_first(&mut xs, 99)
        before * 100 + xs[0]
    "#,
    )
    .expect_number(599.0);
}

#[test]
fn mut_ref_array_mutation_visible_to_caller() {
    ShapeTest::new(
        r#"
        fn fill(arr: &mut Array<int>, val: int) {
            let mut i = 0
            while i < arr.len() {
                arr[i] = val
                i = i + 1
            }
        }
        let mut xs = [0, 0, 0, 0]
        fill(&mut xs, 7)
        xs[0] + xs[1] + xs[2] + xs[3]
    "#,
    )
    .expect_number(28.0);
}

#[test]
fn mixed_ref_and_value_different_args() {
    // Explicit `&mut` out-param plus a consumed read-only value arg.
    ShapeTest::new(
        r#"
        fn copy_first(&target, source: Array<int>) {
            target = source[0]
        }
        let xs = [42]
        let mut result = 0
        copy_first(&mut result, xs)
        result
    "#,
    )
    .expect_number(42.0);
}
