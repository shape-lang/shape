//! Stress tests for break and continue: loop+break, for break, while break,
//! continue in for/while, combined break+continue, break with value,
//! and multiple continue conditions.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 6. Loop + break
// =========================================================================

/// Verifies loop break basic.
#[test]
fn test_loop_break_basic() {
    ShapeTest::new("fn run() {\n    let mut x = 0\n    loop {\n        x = x + 1\n        if x >= 5 {\n            break\n        }\n    }\n    x\n}\nrun()").expect_number(5.0);
}

/// Verifies loop immediate break.
#[test]
fn test_loop_immediate_break() {
    ShapeTest::new(
        "fn run() {\n    let mut x = 0\n    loop {\n        break\n    }\n    x\n}\nrun()",
    )
    .expect_number(0.0);
}

/// Verifies loop break after 10.
#[test]
fn test_loop_break_after_10() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    loop {\n        i = i + 1\n        if i == 10 {\n            break\n        }\n    }\n    i\n}\nrun()").expect_number(10.0);
}

/// Verifies loop sum until break.
#[test]
fn test_loop_sum_until_break() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 1\n    loop {\n        sum = sum + i\n        i = i + 1\n        if i > 10 {\n            break\n        }\n    }\n    sum\n}\nrun()").expect_number(55.0);
}

// =========================================================================
// 8. Break from for loop
// =========================================================================

/// Verifies for break early.
#[test]
fn test_for_break_early() {
    ShapeTest::new("fn run() {\n    let mut last = 0\n    for x in [1, 2, 3, 4, 5] {\n        if x == 3 {\n            break\n        }\n        last = x\n    }\n    last\n}\nrun()").expect_number(2.0);
}

/// Verifies for break first element.
#[test]
fn test_for_break_first_element() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for x in [10, 20, 30] {\n        break\n    }\n    count\n}\nrun()").expect_number(0.0);
}

/// Verifies for range break.
#[test]
fn test_for_range_break() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 0..100 {\n        if i >= 5 {\n            break\n        }\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(10.0);
}

// =========================================================================
// 9. Break from while loop
// =========================================================================

/// Verifies while break midway.
#[test]
fn test_while_break_midway() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    while i < 100 {\n        if i == 7 {\n            break\n        }\n        i = i + 1\n    }\n    i\n}\nrun()").expect_number(7.0);
}

// =========================================================================
// 10. Continue in for loop
// =========================================================================

/// Verifies for continue skip even.
#[test]
fn test_for_continue_skip_even() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] {\n        if i % 2 == 0 {\n            continue\n        }\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(25.0);
}

/// Verifies for continue skip odd.
#[test]
fn test_for_continue_skip_odd() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in [1, 2, 3, 4, 5, 6] {\n        if i % 2 != 0 {\n            continue\n        }\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(12.0);
}

/// Verifies for range continue skip multiples of 3.
#[test]
fn test_for_range_continue_skip_multiples_of_3() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 1..10 {\n        if i % 3 == 0 {\n            continue\n        }\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(27.0);
}

/// Verifies for continue all elements.
#[test]
fn test_for_continue_all_elements() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in [1, 2, 3] {\n        continue\n    }\n    sum\n}\nrun()").expect_number(0.0);
}

// =========================================================================
// 24. Nested break + continue combinations
// =========================================================================

/// Verifies break and continue in same loop.
#[test]
fn test_break_and_continue_in_same_loop() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 0\n    while i < 20 {\n        i = i + 1\n        if i % 3 == 0 {\n            continue\n        }\n        if i > 10 {\n            break\n        }\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(37.0);
}

/// Verifies for break continue combined.
#[test]
fn test_for_break_continue_combined() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for x in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] {\n        if x % 2 == 0 {\n            continue\n        }\n        if x > 7 {\n            break\n        }\n        sum = sum + x\n    }\n    sum\n}\nrun()").expect_number(16.0);
}

// =========================================================================
// 44. Continue skips rest of body
// =========================================================================

/// Verifies continue skips increment.
#[test]
fn test_continue_skips_increment() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in [1, 2, 3, 4, 5] {\n        if i == 3 {\n            continue\n        }\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(12.0);
}

// =========================================================================
// 53. For loop with continue on first and last
// =========================================================================

/// Verifies for continue first last.
#[test]
fn test_for_continue_first_last() {
    ShapeTest::new("fn run() {\n    let arr = [10, 20, 30, 40, 50]\n    let mut sum = 0\n    let mut idx = 0\n    for x in arr {\n        if idx == 0 || idx == 4 {\n            idx = idx + 1\n            continue\n        }\n        sum = sum + x\n        idx = idx + 1\n    }\n    sum\n}\nrun()").expect_number(90.0);
}

// =========================================================================
// 56. Loop with early break on condition
// =========================================================================

/// Verifies find first greater than.
#[test]
fn test_find_first_greater_than() {
    ShapeTest::new("fn run() {\n    let mut found = -1\n    for x in [3, 7, 2, 15, 9, 1] {\n        if x > 10 {\n            found = x\n            break\n        }\n    }\n    found\n}\nrun()").expect_number(15.0);
}

// =========================================================================
// 67. Break preserves state
// =========================================================================

/// Verifies break preserves accumulator.
#[test]
fn test_break_preserves_accumulator() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    let mut i = 0\n    while i < 100 {\n        sum = sum + i\n        if sum > 50 {\n            break\n        }\n        i = i + 1\n    }\n    sum\n}\nrun()").expect_number(55.0);
}

// =========================================================================
// 38. Loop with multiple break conditions
// =========================================================================

/// Verifies multiple break conditions.
#[test]
fn test_multiple_break_conditions() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    let mut sum = 0\n    while i < 1000 {\n        sum = sum + i\n        i = i + 1\n        if sum > 100 {\n            break\n        }\n    }\n    i\n}\nrun()").expect_number(15.0);
}

// =========================================================================
// 81. Break with value from loop expression
// =========================================================================

/// Verifies break with value.
#[test]
fn test_break_with_value() {
    ShapeTest::new(
        "fn run() {\n    let result = loop {\n        break 42\n    }\n    result\n}\nrun()",
    )
    .expect_number(42.0);
}

/// Verifies break with computed value.
#[test]
fn test_break_with_computed_value() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    let result = loop {\n        i = i + 1\n        if i == 5 {\n            break i * 10\n        }\n    }\n    result\n}\nrun()").expect_number(50.0);
}

// =========================================================================
// 85. Loop with multiple continues
// =========================================================================

/// Verifies multiple continue conditions.
#[test]
fn test_multiple_continue_conditions() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 1..20 {\n        if i % 2 == 0 { continue }\n        if i % 5 == 0 { continue }\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(80.0);
}

// =========================================================================
// 16. Early return from loop
// =========================================================================

/// Verifies return from while loop.
#[test]
fn test_return_from_while() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    while i < 100 {\n        if i == 5 {\n            return i\n        }\n        i = i + 1\n    }\n    return -1\n}\nrun()").expect_number(5.0);
}

/// Verifies return from for loop.
#[test]
fn test_return_from_for() {
    ShapeTest::new("fn run() {\n    for x in [10, 20, 30, 40, 50] {\n        if x == 30 {\n            return x\n        }\n    }\n    return -1\n}\nrun()").expect_number(30.0);
}

/// Verifies return from loop keyword.
#[test]
fn test_return_from_loop() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    loop {\n        i = i + 1\n        if i == 7 {\n            return i\n        }\n    }\n}\nrun()").expect_number(7.0);
}

/// Verifies return from nested loop.
#[test]
fn test_return_from_nested_loop() {
    ShapeTest::new("fn run() {\n    let mut i = 0\n    while i < 10 {\n        let mut j = 0\n        while j < 10 {\n            if i == 3 && j == 4 {\n                return i * 10 + j\n            }\n            j = j + 1\n        }\n        i = i + 1\n    }\n    return -1\n}\nrun()").expect_number(34.0);
}

// =========================================================================
// 73. Loop-based linear search
// =========================================================================

/// Verifies linear search found.
#[test]
fn test_linear_search_found() {
    ShapeTest::new("fn run() {\n    let mut idx = -1\n    let mut i = 0\n    for x in [10, 20, 30, 40, 50] {\n        if x == 30 {\n            idx = i\n            break\n        }\n        i = i + 1\n    }\n    idx\n}\nrun()").expect_number(2.0);
}

/// Verifies linear search not found.
#[test]
fn test_linear_search_not_found() {
    ShapeTest::new("fn run() {\n    let mut idx = -1\n    let mut i = 0\n    for x in [10, 20, 30, 40, 50] {\n        if x == 99 {\n            idx = i\n            break\n        }\n        i = i + 1\n    }\n    idx\n}\nrun()").expect_number(-1.0);
}

// =========================================================================
// 103. Loop with early return and accumulator
// =========================================================================

/// Verifies early return preserves result.
#[test]
fn test_early_return_preserves_result() {
    ShapeTest::new("fn find_threshold(limit: int) -> int {\n    let mut sum = 0\n    let mut i = 0\n    while i < 1000 {\n        sum = sum + i\n        if sum >= limit {\n            return i\n        }\n        i = i + 1\n    }\n    return -1\n}\nfn run() {\n    find_threshold(100)\n}\nrun()").expect_number(14.0);
}

// =========================================================================
// 105. Loop with nested function call and break
// =========================================================================

/// Verifies break after function call.
#[test]
fn test_break_after_function_call() {
    ShapeTest::new("fn is_target(x: int) -> bool {\n    x == 42\n}\nfn run() {\n    let mut found = false\n    for x in [10, 20, 42, 50, 60] {\n        if is_target(x) {\n            found = true\n            break\n        }\n    }\n    if found { 1 } else { 0 }\n}\nrun()").expect_number(1.0);
}
