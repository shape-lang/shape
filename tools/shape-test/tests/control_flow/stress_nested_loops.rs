//! Stress tests for nested loops: 2-level and 3-level nesting, break/continue
//! in inner loops, mixed loop types, upper triangular, and matrix patterns.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 12. Nested loops — 2 levels
// =========================================================================

/// Verifies nested while 2 levels.
#[test]
fn test_nested_while_2_levels() {
    ShapeTest::new("fn run() {\n    let mut total = 0\n    let mut i = 0\n    while i < 3 {\n        let mut j = 0\n        while j < 3 {\n            total = total + 1\n            j = j + 1\n        }\n        i = i + 1\n    }\n    total\n}\nrun()").expect_number(9.0);
}

/// Verifies nested for 2 levels sum.
#[test]
fn test_nested_for_2_levels_sum() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in [1, 2, 3] {\n        for j in [10, 20] {\n            sum = sum + i + j\n        }\n    }\n    sum\n}\nrun()").expect_number(102.0);
}

/// Verifies nested for count pairs.
#[test]
fn test_nested_for_count_pairs() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for i in [1, 2, 3] {\n        for j in [4, 5, 6] {\n            count = count + 1\n        }\n    }\n    count\n}\nrun()").expect_number(9.0);
}

/// Verifies nested while multiplication table sum.
#[test]
fn test_nested_while_multiplication_table_sum() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 1\n    while i <= 4 {\n        let mut j = 1\n        while j <= 4 {\n            sum = sum + i * j\n            j = j + 1\n        }\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(100.0);
}

// =========================================================================
// 13. Nested loops — 3 levels
// =========================================================================

/// Verifies nested 3 levels count.
#[test]
fn test_nested_3_levels_count() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    let mut i = 0\n    while i < 3 {\n        let mut j = 0\n        while j < 3 {\n            let mut k = 0\n            while k < 3 {\n                count = count + 1\n                k = k + 1\n            }\n            j = j + 1\n        }\n        i = i + 1\n    }\n    count\n}\nrun()").expect_number(27.0);
}

/// Verifies nested 3 levels sum.
#[test]
fn test_nested_3_levels_sum() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for a in [1, 2] {\n        for b in [10, 20] {\n            for c in [100, 200] {\n                sum = sum + a + b + c\n            }\n        }\n    }\n    sum\n}\nrun()").expect_number(1332.0);
}

// =========================================================================
// 14. Break from inner nested loop only
// =========================================================================

/// Verifies nested break inner only.
#[test]
fn test_nested_break_inner_only() {
    ShapeTest::new("fn run() {\n    let mut total = 0\n    let mut i = 0\n    while i < 3 {\n        let mut j = 0\n        while j < 100 {\n            if j >= 2 {\n                break\n            }\n            total = total + 1\n            j = j + 1\n        }\n        i = i + 1\n    }\n    total\n}\nrun()").expect_number(6.0);
}

/// Verifies nested for break inner.
#[test]
fn test_nested_for_break_inner() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in [1, 2, 3] {\n        for j in [10, 20, 30, 40, 50] {\n            if j > 20 {\n                break\n            }\n            sum = sum + j\n        }\n    }\n    sum\n}\nrun()").expect_number(90.0);
}

// =========================================================================
// 15. Continue in nested loop
// =========================================================================

/// Verifies nested continue inner.
#[test]
fn test_nested_continue_inner() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in [1, 2, 3] {\n        for j in [1, 2, 3, 4] {\n            if j % 2 == 0 {\n                continue\n            }\n            sum = sum + j\n        }\n    }\n    sum\n}\nrun()").expect_number(12.0);
}

// =========================================================================
// 29. Mixed loop types
// =========================================================================

/// Verifies for inside while.
#[test]
fn test_for_inside_while() {
    ShapeTest::new("fn run() {\n    let mut total = 0\n    let mut round = 0\n    while round < 3 {\n        for x in [1, 2, 3] {\n            total = total + x\n        }\n        round = round + 1\n    }\n    total\n}\nrun()").expect_number(18.0);
}

/// Verifies while inside for.
#[test]
fn test_while_inside_for() {
    ShapeTest::new("fn run() {\n    let mut total = 0\n    for x in [2, 3, 4] {\n        let mut n = x\n        while n > 0 {\n            total = total + 1\n            n = n - 1\n        }\n    }\n    total\n}\nrun()").expect_number(9.0);
}

/// Verifies loop inside for.
#[test]
fn test_loop_inside_for() {
    ShapeTest::new("fn run() {\n    let mut total = 0\n    for limit in [3, 5, 2] {\n        let mut count = 0\n        loop {\n            count = count + 1\n            total = total + 1\n            if count >= limit {\n                break\n            }\n        }\n    }\n    total\n}\nrun()").expect_number(10.0);
}

// =========================================================================
// 35. Deeply nested break
// =========================================================================

/// Verifies deep nested break.
#[test]
fn test_deep_nested_break() {
    ShapeTest::new("fn run() {\n    let mut outer_count = 0\n    let mut i = 0\n    while i < 5 {\n        let mut j = 0\n        while j < 5 {\n            let mut k = 0\n            while k < 5 {\n                if k == 2 {\n                    break\n                }\n                k = k + 1\n            }\n            j = j + 1\n        }\n        outer_count = outer_count + 1\n        i = i + 1\n    }\n    outer_count\n}\nrun()").expect_number(5.0);
}

// =========================================================================
// 40. Nested loop: matrix-like summation
// =========================================================================

/// Verifies nested while i times j.
#[test]
fn test_nested_while_i_times_j() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 0\n    while i < 5 {\n        let mut j = 0\n        while j < 5 {\n            sum = sum + i * j\n            j = j + 1\n        }\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(100.0);
}

// =========================================================================
// 55. Nested for loops computing polynomial
// =========================================================================

/// Verifies nested for polynomial.
#[test]
fn test_nested_for_polynomial() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for a in [1, 2] {\n        for b in [1, 2, 3] {\n            sum = sum + a * b\n        }\n    }\n    sum\n}\nrun()").expect_number(18.0);
}

// =========================================================================
// 63. Nested continue: inner loop only
// =========================================================================

/// Verifies nested continue inner only.
#[test]
fn test_nested_continue_inner_only() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 0\n    while i < 3 {\n        let mut j = 0\n        while j < 5 {\n            j = j + 1\n            if j % 2 == 0 {\n                continue\n            }\n            sum = sum + 1\n        }\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(9.0);
}

// =========================================================================
// 71. Nested loop: count pairs where i < j
// =========================================================================

/// Verifies count ordered pairs.
#[test]
fn test_count_ordered_pairs() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    let mut i = 0\n    while i < 5 {\n        let mut j = i + 1\n        while j < 5 {\n            count = count + 1\n            j = j + 1\n        }\n        i = i + 1\n    }\n    count\n}\nrun()").expect_number(10.0);
}

// =========================================================================
// 74. Nested break: outer continues
// =========================================================================

/// Verifies inner break outer continues.
#[test]
fn test_inner_break_outer_continues() {
    ShapeTest::new("fn run() {\n    let mut outer_iterations = 0\n    for i in [1, 2, 3, 4, 5] {\n        outer_iterations = outer_iterations + 1\n        for j in [10, 20, 30] {\n            if j == 20 {\n                break\n            }\n        }\n    }\n    outer_iterations\n}\nrun()").expect_number(5.0);
}

// =========================================================================
// 86. Nested loop: inner depends on outer variable
// =========================================================================

/// Verifies inner depends on outer.
#[test]
fn test_inner_depends_on_outer() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    let mut i = 1\n    while i <= 5 {\n        let mut j = 0\n        while j < i {\n            count = count + 1\n            j = j + 1\n        }\n        i = i + 1\n    }\n    count\n}\nrun()").expect_number(15.0);
}

// =========================================================================
// 88. Nested for with ranges
// =========================================================================

/// Verifies nested for ranges.
#[test]
fn test_nested_for_ranges() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 0..5 {\n        for j in 0..5 {\n            sum = sum + 1\n        }\n    }\n    sum\n}\nrun()").expect_number(25.0);
}

// =========================================================================
// 92. Nested for with break from inner, continue in outer
// =========================================================================

/// Verifies nested break inner continue outer.
#[test]
fn test_nested_break_inner_continue_outer() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for i in [1, 2, 3, 4, 5] {\n        if i % 2 == 0 {\n            continue\n        }\n        for j in [10, 20, 30, 40] {\n            if j > 20 {\n                break\n            }\n            count = count + 1\n        }\n    }\n    count\n}\nrun()").expect_number(6.0);
}

// =========================================================================
// 96. Double loop: upper triangular
// =========================================================================

/// Verifies upper triangular sum.
#[test]
fn test_upper_triangular_sum() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 0\n    while i < 4 {\n        let mut j = i\n        while j < 4 {\n            sum = sum + 1\n            j = j + 1\n        }\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(10.0);
}

// =========================================================================
// 100. Complex: matrix trace via nested loops
// =========================================================================

/// Verifies matrix trace.
#[test]
fn test_matrix_trace() {
    ShapeTest::new("fn run() {\n    let row0 = [1, 2, 3]\n    let row1 = [4, 5, 6]\n    let row2 = [7, 8, 9]\n    let mut trace = 0\n    trace = trace + row0[0]\n    trace = trace + row1[1]\n    trace = trace + row2[2]\n    trace\n}\nrun()").expect_number(15.0);
}

// =========================================================================
// 106. Triple nested for with arrays
// =========================================================================

/// Verifies triple nested for arrays.
#[test]
fn test_triple_nested_for_arrays() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for a in [1, 2] {\n        for b in [3, 4] {\n            for c in [5, 6] {\n                sum = sum + a * b * c\n            }\n        }\n    }\n    sum\n}\nrun()").expect_number(231.0);
}

// =========================================================================
// 64. Complex loop: prime counting (nested loops with break)
// =========================================================================

/// Verifies count primes to 30 using nested trial division.
#[test]
fn test_count_primes_to_30() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    let mut n = 2\n    while n <= 30 {\n        let mut is_prime = true\n        let mut d = 2\n        while d * d <= n {\n            if n % d == 0 {\n                is_prime = false\n                break\n            }\n            d = d + 1\n        }\n        if is_prime {\n            count = count + 1\n        }\n        n = n + 1\n    }\n    count\n}\nrun()").expect_number(10.0);
}
