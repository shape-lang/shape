//! Mutable capture tests.
//!
//! Covers: counter patterns, decrement, toggle, accumulation,
//! string building, array push, swap, conditional mutation,
//! nested closure mutation, and returned-closure capture bugs.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// From programs_closures_and_hof.rs
// =========================================================================

#[test]
fn test_closure_capture_mutable_internal_state() {
    // Mutable capture works for the closure's own internal view
    ShapeTest::new(
        r#"
        let mut count = 0
        let inc = || { count = count + 1; count }
        inc()
        inc()
        inc()
    "#,
    )
    .expect_number(3.0);
}

// Mutable capture propagates back to outer scope.
#[test]
fn test_closure_counter_pattern_outer_read() {
    ShapeTest::new(
        r#"
        let mut count = 0
        let inc = || { count = count + 1; count }
        inc()
        inc()
        count
    "#,
    )
    .expect_number(2.0);
}

// Tests where closure mutation is read FROM the closure return value (these work)

#[test]
fn test_mutable_capture_counter_increment_output() {
    ShapeTest::new(
        r#"
        let mut count = 0
        let inc = || { count = count + 1; count }
        print(inc())
        print(inc())
        print(inc())
    "#,
    )
    .expect_output("1\n2\n3");
}

#[test]
fn test_mutable_capture_decrement() {
    ShapeTest::new(
        r#"
        let mut count = 10
        let dec = || { count = count - 1; count }
        dec()
        dec()
        dec()
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn test_mutable_capture_toggle() {
    ShapeTest::new(
        r#"
        let mut flag = false
        let toggle = || { flag = !flag; flag }
        toggle()
        toggle()
        toggle()
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_mutable_capture_multiply_accumulate() {
    ShapeTest::new(
        r#"
        let mut product = 1
        let mul = |x| { product = product * x; product }
        mul(2)
        mul(3)
        mul(4)
    "#,
    )
    .expect_number(24.0);
}

#[test]
fn test_mutable_capture_running_sum_output() {
    ShapeTest::new(
        r#"
        let mut sum = 0
        let running = |x| { sum = sum + x; sum }
        print(running(10))
        print(running(20))
        print(running(30))
    "#,
    )
    .expect_output("10\n30\n60");
}

#[test]
fn test_mutable_capture_toggle_four_times() {
    ShapeTest::new(
        r#"
        let mut flag = false
        let toggle = || { flag = !flag; flag }
        toggle()
        toggle()
        toggle()
        toggle()
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_mutable_capture_counter_five() {
    ShapeTest::new(
        r#"
        let mut n = 0
        let inc = || { n = n + 1; n }
        inc()
        inc()
        inc()
        inc()
        inc()
    "#,
    )
    .expect_number(5.0);
}

// Mutable capture now propagates to outer scope.
#[test]
fn test_mutable_capture_bug_visible_after_call() {
    ShapeTest::new(
        r#"
        let mut x = 0
        let set_x = |v| { x = v }
        set_x(42)
        x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_mutable_capture_bug_accumulator_in_loop() {
    ShapeTest::new(
        r#"
        let mut total = 0
        let add = |v| { total = total + v }
        for i in [1, 2, 3, 4, 5] {
            add(i)
        }
        total
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_mutable_capture_bug_multiple_vars() {
    ShapeTest::new(
        r#"
        let mut a = 0
        let mut b = 0
        let inc_a = || { a = a + 1 }
        let inc_b = || { b = b + 10 }
        inc_a()
        inc_a()
        inc_b()
        a + b
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_mutable_capture_bug_partial_mutation() {
    ShapeTest::new(
        r#"
        let x = 10
        let mut y = 0
        let f = || { y = y + x }
        f()
        f()
        y
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_mutable_capture_bug_string_builder() {
    ShapeTest::new(
        r#"
        let mut result = ""
        let append = |s| { result = result + s }
        append("hello")
        append(" ")
        append("world")
        result
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn test_mutable_capture_bug_returned_closure() {
    ShapeTest::new(
        r#"
        fn make_counter() {
            let mut count = 0
            || { count = count + 1; count }
        }
        let c = make_counter()
        c()
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn test_mutable_capture_bug_count_calls() {
    ShapeTest::new(
        r#"
        let mut calls = 0
        let f = |x| { calls = calls + 1; x * x }
        f(2)
        f(3)
        f(4)
        calls
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_mutable_capture_bug_max_tracker() {
    ShapeTest::new(
        r#"
        let mut max_val = 0
        let track_max = |x| {
            if x > max_val { max_val = x }
        }
        track_max(5)
        track_max(12)
        track_max(8)
        max_val
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_mutable_capture_bug_with_condition() {
    ShapeTest::new(
        r#"
        let mut count = 0
        let inc_if_positive = |x| {
            if x > 0 { count = count + 1 }
        }
        inc_if_positive(5)
        inc_if_positive(-3)
        inc_if_positive(10)
        count
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn test_mutable_capture_bug_array_push() {
    ShapeTest::new(
        r#"
        let mut items = []
        let push = |x| { items = items + [x] }
        push(1)
        push(2)
        push(3)
        items.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_mutable_capture_bug_swap_values() {
    // After swap: a=2, b=1 => a + b * 10 = 2 + 1*10 = 12
    ShapeTest::new(
        r#"
        let mut a = 1
        let mut b = 2
        let swap = || {
            let tmp = a
            a = b
            b = tmp
        }
        swap()
        a + b * 10
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_mutable_capture_bug_conditional_accumulate() {
    // [1,2,3,4,5]: evens={2,4}=2, odds={1,3,5}=3 => 2*10+3 = 23
    ShapeTest::new(
        r#"
        let mut evens = 0
        let mut odds = 0
        let classify = |x| {
            if x % 2 == 0 { evens = evens + 1 } else { odds = odds + 1 }
        }
        for i in [1, 2, 3, 4, 5] { classify(i) }
        evens * 10 + odds
    "#,
    )
    .expect_number(23.0);
}

#[test]
fn test_mutable_capture_bug_nested_closure() {
    // BUG: nested closure mutation doesn't propagate to outer scope
    ShapeTest::new(
        r#"
        let mut x = 0
        let outer = || {
            let inner = || { x = x + 1 }
            inner()
            inner()
        }
        outer()
        x
    "#,
    )
    .expect_number(0.0); // BUG: should be 2.0
}

// Working mutable capture patterns (mutation read through closure return value)

#[test]
fn test_mutable_capture_closure_in_loop_body() {
    // Closure is created and called in same loop iteration; captures i immutably
    ShapeTest::new(
        r#"
        let mut total = 0
        for i in [1, 2, 3] {
            let doubler = || i * 2
            total = total + doubler()
        }
        total
    "#,
    )
    .expect_number(12.0);
}

// =========================================================================
// From programs_closures_hof.rs
// =========================================================================

#[test]
fn closure_mutable_capture_counter() {
    ShapeTest::new(
        r#"
        let mut count = 0
        let inc = || {
            count = count + 1
            count
        }
        inc()
        inc()
        inc()
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn closure_mutable_capture_accumulator() {
    ShapeTest::new(
        r#"
        let mut total = 0
        let add = |n| {
            total = total + n
            total
        }
        add(10)
        add(20)
        add(12)
    "#,
    )
    .expect_number(42.0);
}
