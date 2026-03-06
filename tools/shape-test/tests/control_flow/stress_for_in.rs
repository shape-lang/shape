//! Stress tests for for-in loops: array iteration, range iteration (exclusive
//! and inclusive), string iteration, variable bounds, and edge cases.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 4. For-in with array literal
// =========================================================================

/// Verifies for-in array sum.
#[test]
fn test_for_in_array_sum() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for x in [1, 2, 3, 4, 5] {\n        sum = sum + x\n    }\n    sum\n}\nrun()").expect_number(15.0);
}

/// Verifies for-in array product.
#[test]
fn test_for_in_array_product() {
    ShapeTest::new("fn run() {\n    let mut prod = 1\n    for x in [2, 3, 4] {\n        prod = prod * x\n    }\n    prod\n}\nrun()").expect_number(24.0);
}

/// Verifies for-in empty array.
#[test]
fn test_for_in_empty_array() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for x in [] {\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(0.0);
}

/// Verifies for-in single element array.
#[test]
fn test_for_in_single_element_array() {
    ShapeTest::new("fn run() {\n    let mut val = 0\n    for x in [42] {\n        val = x\n    }\n    val\n}\nrun()").expect_number(42.0);
}

/// Verifies for-in array variable.
#[test]
fn test_for_in_array_variable() {
    ShapeTest::new("fn run() {\n    let arr = [10, 20, 30]\n    let mut sum = 0\n    for x in arr {\n        sum = sum + x\n    }\n    sum\n}\nrun()").expect_number(60.0);
}

/// Verifies for-in array count elements.
#[test]
fn test_for_in_array_count_elements() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for x in [10, 20, 30, 40, 50] {\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(5.0);
}

/// Verifies for-in array last element.
#[test]
fn test_for_in_array_last_element() {
    ShapeTest::new("fn run() {\n    let mut last = 0\n    for x in [1, 2, 3, 4, 5] {\n        last = x\n    }\n    last\n}\nrun()").expect_number(5.0);
}

// =========================================================================
// 5. For-in with range
// =========================================================================

/// Verifies for-in range sum 0 to 5.
#[test]
fn test_for_in_range_sum_0_to_5() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 0..5 {\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(10.0);
}

/// Verifies for-in range sum 1 to 10.
#[test]
fn test_for_in_range_sum_1_to_10() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 1..11 {\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(55.0);
}

/// Verifies for-in range count.
#[test]
fn test_for_in_range_count() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for i in 0..7 {\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(7.0);
}

/// Verifies for-in range empty (same start/end).
#[test]
fn test_for_in_range_empty() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for i in 5..5 {\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(0.0);
}

/// Verifies for-in range empty reverse.
#[test]
fn test_for_in_range_empty_reverse() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for i in 10..5 {\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(0.0);
}

/// Verifies for-in range single element.
#[test]
fn test_for_in_range_single_element() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 3..4 {\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(3.0);
}

/// Verifies for-in range inclusive.
#[test]
fn test_for_in_range_inclusive() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 1..=5 {\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(15.0);
}

/// Verifies for-in range inclusive single.
#[test]
fn test_for_in_range_inclusive_single() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 5..=5 {\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(5.0);
}

// =========================================================================
// 28. Loop over string characters (for-in on string)
// =========================================================================

/// Verifies for-in string count chars.
#[test]
fn test_for_in_string_count_chars() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for ch in \"hello\" {\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(5.0);
}

/// Verifies for-in empty string.
#[test]
fn test_for_in_empty_string() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for ch in \"\" {\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(0.0);
}

// =========================================================================
// 30. Range with non-zero start
// =========================================================================

/// Verifies for-in range 5 to 10.
#[test]
fn test_for_in_range_5_to_10() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 5..10 {\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(35.0);
}

/// Verifies for-in range 100 to 105.
#[test]
fn test_for_in_range_100_to_105() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 100..105 {\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(510.0);
}

// =========================================================================
// 34. For loop with array of mixed operations
// =========================================================================

/// Verifies for alternating add sub.
#[test]
fn test_for_alternating_add_sub() {
    ShapeTest::new("fn run() {\n    let ops = [1, -1, 2, -2, 3, -3]\n    let mut sum = 0\n    for x in ops {\n        sum = sum + x\n    }\n    sum\n}\nrun()").expect_number(0.0);
}

// =========================================================================
// 41. For with complex expressions
// =========================================================================

/// Verifies for with negative elements.
#[test]
fn test_for_with_negative_elements() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for x in [-5, -3, -1, 1, 3, 5] {\n        sum = sum + x\n    }\n    sum\n}\nrun()").expect_number(0.0);
}

/// Verifies for absolute value sum.
#[test]
fn test_for_absolute_value_sum() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for x in [-5, -3, 0, 3, 5] {\n        if x < 0 {\n            sum = sum - x\n        } else {\n            sum = sum + x\n        }\n    }\n    sum\n}\nrun()").expect_number(16.0);
}

// =========================================================================
// 48. Counting specific values
// =========================================================================

/// Verifies count specific value.
#[test]
fn test_count_specific_value() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for x in [1, 2, 3, 2, 1, 2, 3, 2, 1] {\n        if x == 2 {\n            count = count + 1\n        }\n    }\n    count\n}\nrun()").expect_number(4.0);
}

// =========================================================================
// 57. Inclusive range: verify inclusive end
// =========================================================================

/// Verifies inclusive range boundary.
#[test]
fn test_inclusive_range_boundary() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for i in 0..=10 {\n        count = count + 1\n    }\n    count\n}\nrun()").expect_number(11.0);
}

// =========================================================================
// 58. Exclusive range: verify exclusive end
// =========================================================================

/// Verifies exclusive range boundary.
#[test]
fn test_exclusive_range_boundary() {
    ShapeTest::new("fn run() {\n    let mut last = -1\n    for i in 0..5 {\n        last = i\n    }\n    last\n}\nrun()").expect_number(4.0);
}

// =========================================================================
// 60. For loop with zero in array
// =========================================================================

/// Verifies for with zeros.
#[test]
fn test_for_with_zeros() {
    ShapeTest::new("fn run() {\n    let mut count = 0\n    for x in [0, 0, 0, 1, 0] {\n        if x == 0 {\n            count = count + 1\n        }\n    }\n    count\n}\nrun()").expect_number(4.0);
}

// =========================================================================
// 65. Loop with no-op body (just counting)
// =========================================================================

/// Verifies range just count.
#[test]
fn test_range_just_count() {
    ShapeTest::new("fn run() {\n    let mut c = 0\n    for i in 0..50 {\n        c = c + 1\n    }\n    c\n}\nrun()").expect_number(50.0);
}

// =========================================================================
// 70. For loop: double each element
// =========================================================================

/// Verifies for double elements.
#[test]
fn test_for_double_elements() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for x in [5, 10, 15, 20] {\n        sum = sum + x * 2\n    }\n    sum\n}\nrun()").expect_number(100.0);
}

// =========================================================================
// 76. For range: accumulate index squared
// =========================================================================

/// Verifies range index squared.
#[test]
fn test_range_index_squared() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 0..10 {\n        sum = sum + i * i\n    }\n    sum\n}\nrun()").expect_number(285.0);
}

// =========================================================================
// 82. Loop over large range
// =========================================================================

/// Verifies large range sum.
#[test]
fn test_large_range_sum() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 0..100 {\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(4950.0);
}

// =========================================================================
// 84. Range with variable bounds
// =========================================================================

/// Verifies range variable bounds.
#[test]
fn test_range_variable_bounds() {
    ShapeTest::new("fn run() {\n    let start = 5\n    let end = 10\n    let mut sum = 0\n    for i in start..end {\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(35.0);
}

// =========================================================================
// 90. For with large array
// =========================================================================

/// Verifies for large literal array.
#[test]
fn test_for_large_literal_array() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for x in [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20] {\n        sum = sum + x\n    }\n    sum\n}\nrun()").expect_number(210.0);
}

// =========================================================================
// 102. For-in over range with inclusive bounds
// =========================================================================

/// Verifies range inclusive sum 1 to 100.
#[test]
fn test_range_inclusive_sum_1_to_100() {
    ShapeTest::new("fn run() {\n    let mut sum = 0\n    for i in 1..=100 {\n        sum = sum + i\n    }\n    sum\n}\nrun()").expect_number(5050.0);
}

// =========================================================================
// 32. Loop variable does not leak scope
// =========================================================================

/// Verifies for variable scope.
#[test]
fn test_for_variable_scope() {
    ShapeTest::new("fn run() {\n    let mut last = 0\n    for x in [1, 2, 3] {\n        last = x\n    }\n    last\n}\nrun()").expect_number(3.0);
}

// =========================================================================
// 52. Range cumulative product
// =========================================================================

/// Verifies range cumulative product.
#[test]
fn test_range_cumulative_product() {
    ShapeTest::new("fn run() {\n    let mut prod = 1\n    for i in 1..8 {\n        prod = prod * i\n    }\n    prod\n}\nrun()").expect_number(5040.0);
}
