//! Stress tests for while loops: basic counting, accumulators, floating point,
//! boolean conditions, countdown patterns, zero iterations, and convergence.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. While loop — basic counting
// =========================================================================

/// Verifies while loop counting to 5.
#[test]
fn test_while_count_to_5() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    while i < 5 {\n        i = i + 1\n    }\n    i\n}\nrun()").expect_number(5.0);
}

/// Verifies while loop counting to 10.
#[test]
fn test_while_count_to_10() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    while i < 10 {\n        i = i + 1\n    }\n    i\n}\nrun()").expect_number(10.0);
}

/// Verifies while loop zero iterations.
#[test]
fn test_while_zero_iterations() {
    ShapeTest::new("fn run() {\n    let mut i = 100\n    while i < 0 {\n        i = i + 1\n    }\n    i\n}\nrun()").expect_number(100.0);
}

/// Verifies while loop single iteration via flag.
#[test]
fn test_while_single_iteration() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    let mut flag = true\n    while flag {\n        count = count + 1\n        flag = false\n    }\n    count\n}\nrun()").expect_number(1.0);
}

/// Verifies while loop sum 1 to 100.
#[test]
fn test_while_sum_1_to_100() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 1\n    while i <= 100 {\n        sum = sum + i\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(5050.0);
}

// =========================================================================
// 2. While loop — accumulator patterns
// =========================================================================

/// Verifies while factorial of 10.
#[test]
fn test_while_factorial_10() {
    ShapeTest::new("fn run() {\n    let mut fact = 1\n    let mut i = 2\n    while i <= 10 {\n        fact = fact * i\n        i = i + 1\n    }\n    fact\n}\nrun()").expect_number(3628800.0);
}

/// Verifies while power of 2.
#[test]
fn test_while_power_of_2() {
    ShapeTest::new("fn run() {\n    let mut val = 1\n    let mut i = 0\n    while i < 10 {\n        val = val * 2\n        i = i + 1\n    }\n    val\n}\nrun()").expect_number(1024.0);
}

/// Verifies while sum of squares.
#[test]
fn test_while_sum_of_squares() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 1\n    while i <= 5 {\n        sum = sum + i * i\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(55.0);
}

/// Verifies while countdown.
#[test]
fn test_while_countdown() {
    ShapeTest::new("fn run() {\n    let mut x = 10\n    let mut steps = 0\n    while x > 0 {\n        x = x - 1\n        steps = steps + 1\n    }\n    steps\n}\nrun()").expect_number(10.0);
}

/// Verifies while fibonacci iterative.
#[test]
fn test_while_fibonacci_iterative() {
    ShapeTest::new("fn run() {\n    let mut a = 0\n    let mut b = 1\n    let mut i = 0\n    while i < 10 {\n        let temp = b\n        b = a + b\n        a = temp\n        i = i + 1\n    }\n    a\n}\nrun()").expect_number(55.0);
}

// =========================================================================
// 3. While loop — floating point
// =========================================================================

/// Verifies while float accumulator.
#[test]
fn test_while_float_accumulator() {
    ShapeTest::new("fn run() {\n    let mut sum = 0.0\n    let mut i = 0\n    while i < 100 {\n        sum = sum + 1.0\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(100.0);
}

/// Verifies while float decrement halving.
#[test]
fn test_while_float_decrement() {
    ShapeTest::new("fn run() {\n    let mut val = 10.0\n    let mut count = 0\n    while val > 0.5 {\n        val = val / 2.0\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(5.0);
}

// =========================================================================
// 7. While true + break pattern
// =========================================================================

/// Verifies while true with break.
#[test]
fn test_while_true_break() {
    ShapeTest::new("fn run() {\n    let mut x = 0\n    while true {\n        x = x + 1\n        if x >= 3 {\n            break\n        }\n    }\n    x\n}\nrun()").expect_number(3.0);
}

/// Verifies while true immediate break.
#[test]
fn test_while_true_immediate_break() {
    ShapeTest::new("fn run() {\n    let mut x = 42\n    while true {\n        break\n    }\n    x\n}\nrun()").expect_number(42.0);
}

// =========================================================================
// 11. Continue in while loop
// =========================================================================

/// Verifies while continue skip even.
#[test]
fn test_while_continue_skip_even() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 0\n    while i < 10 {\n        i = i + 1\n        if i % 2 == 0 {\n            continue\n        }\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(25.0);
}

// =========================================================================
// 23. Loop with boolean conditions
// =========================================================================

/// Verifies while boolean variable condition.
#[test]
fn test_while_boolean_variable() {
    ShapeTest::new("fn run() {\n    let mut found = false\n    let mut i = 0\n    while !found {\n        i = i + 1\n        if i == 8 {\n            found = true\n        }\n    }\n    i\n}\nrun()").expect_number(8.0);
}

/// Verifies while complex compound condition.
#[test]
fn test_while_complex_condition() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    let mut sum = 0\n    while i < 20 && sum < 50 {\n        sum = sum + i\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(55.0);
}

// =========================================================================
// 33. While with decrement
// =========================================================================

/// Verifies while decrement to zero.
#[test]
fn test_while_decrement_to_zero() {
    ShapeTest::new("fn run() {\n    let mut n = 50\n    let mut steps = 0\n    while n > 0 {\n        n = n - 3\n        steps = steps + 1\n    }\n    steps\n}\nrun()").expect_number(17.0);
}

/// Verifies while halving.
#[test]
fn test_while_halving() {
    ShapeTest::new("fn run() {\n    let mut n = 1024\n    let mut steps = 0\n    while n > 1 {\n        n = n / 2\n        steps = steps + 1\n    }\n    steps\n}\nrun()").expect_number(10.0);
}

// =========================================================================
// 42. While false — zero iterations
// =========================================================================

/// Verifies while false body never executes.
#[test]
fn test_while_false_body() {
    ShapeTest::new("fn run() {\n    let mut x = 42\n    while false {\n        x = 0\n    }\n    x\n}\nrun()").expect_number(42.0);
}

// =========================================================================
// 45. While loop with float condition
// =========================================================================

/// Verifies while float condition.
#[test]
fn test_while_float_condition() {
    ShapeTest::new("fn run() {\n    let mut x = 0.0\n    let mut count = 0\n    while x < 5.5 {\n        x = x + 1.0\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(6.0);
}

// =========================================================================
// 54. While loop with compound condition using OR
// =========================================================================

/// Verifies while OR condition.
#[test]
fn test_while_or_condition() {
    ShapeTest::new("fn run() {\n    let mut x = 0\n    let mut y = 100\n    let mut steps = 0\n    while x < 50 || y > 50 {\n        x = x + 1\n        y = y - 1\n        steps = steps + 1\n    }\n    steps\n}\nrun()").expect_number(50.0);
}

// =========================================================================
// 61. While loop — exactly one iteration
// =========================================================================

/// Verifies while exactly one iteration.
#[test]
fn test_while_exactly_one() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    let mut i = 0\n    while i < 1 {\n        count = count + 1\n        i = i + 1\n    }\n    count\n}\nrun()").expect_number(1.0);
}

// =========================================================================
// 62. Loop with increasing step
// =========================================================================

/// Verifies while increasing step.
#[test]
fn test_while_increasing_step() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    let mut step = 1\n    let mut count = 0\n    while i < 100 {\n        i = i + step\n        step = step + 1\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(14.0);
}

// =========================================================================
// 66. While loop with > vs >=
// =========================================================================

/// Verifies while gt vs gte difference.
#[test]
fn test_while_gt_vs_gte() {
    ShapeTest::new("fn run() {\n    let mut a = 0\n    let mut i = 10\n    while i > 0 {\n        a = a + 1\n        i = i - 1\n    }\n    let mut b = 0\n    i = 10\n    while i >= 0 {\n        b = b + 1\n        i = i - 1\n    }\n    b - a\n}\nrun()").expect_number(1.0);
}

// =========================================================================
// 77. While loop — counting down by 2
// =========================================================================

/// Verifies countdown by 2.
#[test]
fn test_countdown_by_2() {
    ShapeTest::new("fn run() {\n    let mut n = 20\n    let mut count = 0\n    while n > 0 {\n        n = n - 2\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(10.0);
}

// =========================================================================
// 89. While with pre-check vs post-check pattern
// =========================================================================

/// Verifies while precheck pattern.
#[test]
fn test_while_precheck() {
    ShapeTest::new("fn run() {\n    let mut i = 10\n    let mut count = 0\n    while i > 0 {\n        count = count + 1\n        i = i - 1\n    }\n    count\n}\nrun()").expect_number(10.0);
}

/// Verifies loop postcheck pattern.
#[test]
fn test_loop_postcheck() {
    ShapeTest::new("fn run() {\n    let mut i = 10\n    let mut count = 0\n    loop {\n        count = count + 1\n        i = i - 1\n        if i <= 0 {\n            break\n        }\n    }\n    count\n}\nrun()").expect_number(10.0);
}

// =========================================================================
// 107. While loop: convergence test
// =========================================================================

/// Verifies convergence loop.
#[test]
fn test_convergence_loop() {
    ShapeTest::new("fn run() {\n    let mut x = 100.0\n    let mut steps = 0\n    while x > 1.0 {\n        x = x * 0.9\n        steps = steps + 1\n    }\n    steps\n}\nrun()").expect_number(44.0);
}
