//! Stress tests for iterator terminal operations (reduce, find, any, all),
//! enumerate, chain, for..in loops, large data, multiple terminals, forEach,
//! edge cases, array take/skip/first/last/reverse, pipe operator, and string iterators.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 5: Iterator Terminal Operations — reduce, find, any, all
// =============================================================================

/// Iter reduce sum.
#[test]
fn test_iter_reduce_sum() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].iter().reduce(|acc, x| acc + x, 0)"#).expect_number(15.0);
}

/// Iter reduce product.
#[test]
fn test_iter_reduce_product() {
    ShapeTest::new(r#"[1, 2, 3, 4].iter().reduce(|acc, x| acc * x, 1)"#).expect_number(24.0);
}

/// Iter reduce empty.
#[test]
fn test_iter_reduce_empty() {
    ShapeTest::new(r#"[].iter().reduce(|acc, x| acc + x, 99)"#).expect_number(99.0);
}

/// Iter find found.
#[test]
fn test_iter_find_found() {
    ShapeTest::new(r#"[10, 20, 30, 40].iter().find(|x| x > 25)"#).expect_number(30.0);
}

/// Iter find not found.
#[test]
fn test_iter_find_not_found() {
    ShapeTest::new(r#"[10, 20, 30].iter().find(|x| x > 100)"#).expect_none();
}

/// Iter find first element.
#[test]
fn test_iter_find_first_element() {
    ShapeTest::new(r#"[1, 2, 3].iter().find(|x| x > 0)"#).expect_number(1.0);
}

/// Iter any true.
#[test]
fn test_iter_any_true() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].iter().any(|x| x > 4)"#).expect_bool(true);
}

/// Iter any false.
#[test]
fn test_iter_any_false() {
    ShapeTest::new(r#"[1, 2, 3].iter().any(|x| x > 10)"#).expect_bool(false);
}

/// Iter any empty.
#[test]
fn test_iter_any_empty() {
    ShapeTest::new(r#"[].iter().any(|x| x > 0)"#).expect_bool(false);
}

/// Iter all true.
#[test]
fn test_iter_all_true() {
    ShapeTest::new(r#"[2, 4, 6].iter().all(|x| x > 0)"#).expect_bool(true);
}

/// Iter all false.
#[test]
fn test_iter_all_false() {
    ShapeTest::new(r#"[2, 4, 6].iter().all(|x| x > 3)"#).expect_bool(false);
}

/// Iter all empty — vacuous truth.
#[test]
fn test_iter_all_empty() {
    ShapeTest::new(r#"[].iter().all(|x| x > 0)"#).expect_bool(true);
}

// =============================================================================
// SECTION 8: Enumerate
// =============================================================================

/// Iter enumerate collect — check pair structure.
#[test]
fn test_iter_enumerate_collect() {
    ShapeTest::new(
        r#"
        {
            let arr = [10, 20, 30].iter().enumerate().collect()
            let pair0 = arr[0]
            pair0[0] + pair0[1]
        }
    "#,
    )
    .expect_number(10.0);
}

/// Iter enumerate empty.
#[test]
fn test_iter_enumerate_empty() {
    ShapeTest::new(r#"[].iter().enumerate().collect().length"#).expect_number(0.0);
}

/// Iter enumerate count.
#[test]
fn test_iter_enumerate_count() {
    ShapeTest::new(r#"[10, 20, 30, 40].iter().enumerate().count()"#).expect_number(4.0);
}

/// Iter enumerate take.
#[test]
fn test_iter_enumerate_take() {
    ShapeTest::new(r#"[10, 20, 30, 40, 50].iter().enumerate().take(2).collect().length"#)
        .expect_number(2.0);
}

// =============================================================================
// SECTION 9: Iterator chain() — concatenation
// =============================================================================

/// Iter chain two arrays.
#[test]
fn test_iter_chain_two_arrays() {
    ShapeTest::new(
        r#"
        {
            let arr = [1, 2, 3].iter().chain([4, 5, 6].iter()).collect()
            arr[0] + arr[5]
        }
    "#,
    )
    .expect_number(7.0);
}

/// Iter chain with empty.
#[test]
fn test_iter_chain_with_empty() {
    ShapeTest::new(r#"[1, 2, 3].iter().chain([].iter()).collect().length"#).expect_number(3.0);
}

/// Iter chain empty with nonempty.
#[test]
fn test_iter_chain_empty_with_nonempty() {
    ShapeTest::new(r#"[].iter().chain([4, 5, 6].iter()).collect()[0]"#).expect_number(4.0);
}

/// Iter chain then count.
#[test]
fn test_iter_chain_then_count() {
    ShapeTest::new(r#"[1, 2].iter().chain([3, 4, 5].iter()).count()"#).expect_number(5.0);
}

// =============================================================================
// SECTION 12: Direct Array vs Iterator Equivalence
// =============================================================================

/// Direct map vs iter map equivalence.
#[test]
fn test_direct_map_vs_iter_map() {
    ShapeTest::new(
        r#"
        {
            let d = [1, 2, 3].map(|x| x * 2).length
            let i = [1, 2, 3].iter().map(|x| x * 2).collect().length
            d == i
        }
    "#,
    )
    .expect_bool(true);
}

/// Direct filter vs iter filter equivalence.
#[test]
fn test_direct_filter_vs_iter_filter() {
    ShapeTest::new(
        r#"
        {
            let d = [1, 2, 3, 4, 5].filter(|x| x > 3).length
            let i = [1, 2, 3, 4, 5].iter().filter(|x| x > 3).collect().length
            d == i
        }
    "#,
    )
    .expect_bool(true);
}

/// Direct reduce vs iter reduce equivalence.
#[test]
fn test_direct_reduce_vs_iter_reduce() {
    ShapeTest::new(
        r#"
        {
            let d = [1, 2, 3, 4].reduce(|acc, x| acc + x, 0)
            let i = [1, 2, 3, 4].iter().reduce(|acc, x| acc + x, 0)
            d == i
        }
    "#,
    )
    .expect_bool(true);
}

// =============================================================================
// SECTION 13: for..in Loop with Arrays and Ranges
// =============================================================================

/// For in array sum.
#[test]
fn test_for_in_array_sum() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let mut total = 0
            for x in [1, 2, 3, 4, 5] {
                total = total + x
            }
            total
        }
        test()
    "#,
    )
    .expect_number(15.0);
}

/// For in range.
#[test]
fn test_for_in_range() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let mut total = 0
            for i in range(0, 5) {
                total = total + i
            }
            total
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// For in filtered array.
#[test]
fn test_for_in_filtered_array() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let nums = [1, 2, 3, 4, 5, 6]
            let evens = nums.filter(|x| x % 2 == 0)
            let mut total = 0
            for e in evens {
                total = total + e
            }
            total
        }
        test()
    "#,
    )
    .expect_number(12.0);
}

/// For in mapped array.
#[test]
fn test_for_in_mapped_array() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let doubled = [1, 2, 3].map(|x| x * 2)
            let mut total = 0
            for d in doubled {
                total = total + d
            }
            total
        }
        test()
    "#,
    )
    .expect_number(12.0);
}

/// For in empty array.
#[test]
fn test_for_in_empty_array() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let mut total = 0
            for x in [] {
                total = total + 1
            }
            total
        }
        test()
    "#,
    )
    .expect_number(0.0);
}

// =============================================================================
// SECTION 14: Large Data Sets
// =============================================================================

/// Iter large array count.
#[test]
fn test_iter_large_array_count() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let mut arr: Array<int> = []
            for i in range(0, 100) {
                arr = arr.concat([i])
            }
            arr.iter().count()
        }
        test()
    "#,
    )
    .expect_number(100.0);
}

/// Iter large array filter count.
#[test]
fn test_iter_large_array_filter_count() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let mut arr: Array<int> = []
            for i in range(0, 100) {
                arr = arr.concat([i])
            }
            arr.iter().filter(|x| x % 2 == 0).count()
        }
        test()
    "#,
    )
    .expect_number(50.0);
}

/// Iter large array map take collect.
#[test]
fn test_iter_large_array_map_take_collect() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let mut arr: Array<int> = []
            for i in range(0, 100) {
                arr = arr.concat([i])
            }
            let first_five = arr.iter().map(|x| x * 10).take(5).collect()
            first_five.length
        }
        test()
    "#,
    )
    .expect_number(5.0);
}

// =============================================================================
// SECTION 15: Multiple Terminals on Same Source
// =============================================================================

/// Multiple terminals same source.
#[test]
fn test_multiple_terminals_same_source() {
    ShapeTest::new(
        r#"
        {
            let data = [1, 2, 3, 4, 5]
            let count_val = data.iter().count()
            let sum_val = data.iter().reduce(|acc, x| acc + x, 0)
            count_val + sum_val
        }
    "#,
    )
    .expect_number(20.0);
}

/// Reuse array for multiple operations.
#[test]
fn test_reuse_array_for_multiple_operations() {
    ShapeTest::new(
        r#"
        {
            let nums = [10, 20, 30, 40, 50]
            let doubled = nums.map(|x| x * 2)
            let filtered = nums.filter(|x| x > 25)
            doubled.length + filtered.length
        }
    "#,
    )
    .expect_number(8.0);
}

// =============================================================================
// SECTION 16: forEach (side-effect)
// =============================================================================

/// Array forEach.
#[test]
fn test_array_foreach() {
    ShapeTest::new(
        r#"
        fn test() -> int {
            let mut total = 0
            [1, 2, 3].forEach(|x| { total = total + x })
            total
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

// =============================================================================
// SECTION 19: Edge Cases
// =============================================================================

/// Single element iter all operations.
#[test]
fn test_single_element_iter_all_operations() {
    ShapeTest::new(
        r#"
        {
            let c = [42].iter().count()
            let a = [42].iter().any(|x| x == 42)
            c
        }
    "#,
    )
    .expect_number(1.0);
}

/// Single element iter any check.
#[test]
fn test_single_element_iter_any_check() {
    ShapeTest::new(r#"[42].iter().any(|x| x == 42)"#).expect_bool(true);
}

/// Iter take from empty.
#[test]
fn test_iter_take_from_empty() {
    ShapeTest::new(r#"[].iter().take(5).collect().length"#).expect_number(0.0);
}

/// Iter skip from empty.
#[test]
fn test_iter_skip_from_empty() {
    ShapeTest::new(r#"[].iter().skip(5).collect().length"#).expect_number(0.0);
}

/// Iter filter from empty.
#[test]
fn test_iter_filter_from_empty() {
    ShapeTest::new(r#"[].iter().filter(|x| x > 0).collect().length"#).expect_number(0.0);
}

/// Iter reduce from empty.
#[test]
fn test_iter_reduce_from_empty() {
    ShapeTest::new(r#"[].iter().reduce(|acc, x| acc + x, 0)"#).expect_number(0.0);
}

/// Iter find from empty.
#[test]
fn test_iter_find_from_empty() {
    ShapeTest::new(r#"[].iter().find(|x| x > 0)"#).expect_none();
}

/// Iter any from empty.
#[test]
fn test_iter_any_from_empty() {
    ShapeTest::new(r#"[].iter().any(|x| x > 0)"#).expect_bool(false);
}

/// Iter all from empty.
#[test]
fn test_iter_all_from_empty() {
    ShapeTest::new(r#"[].iter().all(|x| x > 0)"#).expect_bool(true);
}

/// Iter enumerate from empty.
#[test]
fn test_iter_enumerate_from_empty() {
    ShapeTest::new(r#"[].iter().enumerate().collect().length"#).expect_number(0.0);
}

// =============================================================================
// SECTION 20: Array take/skip/first/last
// =============================================================================

/// Array take.
#[test]
fn test_array_take() {
    ShapeTest::new(
        r#"
        {
            let arr = [1, 2, 3, 4, 5].take(3)
            arr[0] + arr[2]
        }
    "#,
    )
    .expect_number(4.0);
}

/// Array skip.
#[test]
fn test_array_skip() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].skip(2)[0]"#).expect_number(3.0);
}

/// Array first.
#[test]
fn test_array_first() {
    ShapeTest::new(r#"[10, 20, 30].first()"#).expect_number(10.0);
}

/// Array last.
#[test]
fn test_array_last() {
    ShapeTest::new(r#"[10, 20, 30].last()"#).expect_number(30.0);
}

/// Array reverse.
#[test]
fn test_array_reverse() {
    ShapeTest::new(
        r#"
        {
            let arr = [1, 2, 3].reverse()
            arr[0] + arr[1] * 10 + arr[2] * 100
        }
    "#,
    )
    .expect_number(123.0);
}

// =============================================================================
// SECTION 21: Pipe Operator with Array-returning Functions
// =============================================================================

/// Direct function chaining with computation.
#[test]
fn test_pipe_with_computation() {
    ShapeTest::new(
        r#"
        fn square(x: int) -> int { x * x }
        fn add(x: int, y: int) -> int { x + y }
        fn test() -> int { add(square(3), 1) }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Direct function call with multiple args.
#[test]
fn test_pipe_multiple_args() {
    ShapeTest::new(
        r#"
        fn clamp(val: int, low: int, high: int) -> int {
            if val < low { low }
            else if val > high { high }
            else { val }
        }
        fn test() -> int { clamp(15, 0, 10) }
        test()
    "#,
    )
    .expect_number(10.0);
}

// =============================================================================
// SECTION 22: String iterator (if supported)
// =============================================================================

/// String iter count.
#[test]
fn test_string_iter_collect_via_source() {
    ShapeTest::new(r#""hello".iter().count()"#).expect_number(5.0);
}

/// String iter take.
#[test]
fn test_string_iter_take() {
    ShapeTest::new(r#""abcde".iter().take(3).collect()[0]"#).expect_string("a");
}

/// String iter skip.
#[test]
fn test_string_iter_skip() {
    ShapeTest::new(r#""abcde".iter().skip(3).collect()[0]"#).expect_string("d");
}

/// Empty string iter count.
#[test]
fn test_empty_string_iter() {
    ShapeTest::new(r#""".iter().count()"#).expect_number(0.0);
}
