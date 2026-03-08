//! Stress tests for direct array methods (map, filter, reduce, find, some, every)
//! and iterator lazy map/filter operations.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 1: Direct Array Methods — map, filter, reduce, find, some, every
// =============================================================================

/// Array map basic — check length and first element.
#[test]
fn test_array_map_basic() {
    ShapeTest::new(r#"
        fn test() -> int {
            let arr = [1, 2, 3].map(|x| x * 2)
            arr[0] + arr[1] + arr[2]
        }
        test()
    "#).expect_number(12.0);
}

/// Array map identity.
#[test]
fn test_array_map_identity() {
    ShapeTest::new(r#"
        fn test() -> int {
            let arr = [10, 20, 30].map(|x| x)
            arr[0]
        }
        test()
    "#).expect_number(10.0);
}

/// Array map to bool.
#[test]
fn test_array_map_to_bool() {
    ShapeTest::new(r#"
        fn test() -> bool {
            let arr = [1, 2, 3].map(|x| x > 1)
            arr[0]
        }
        test()
    "#).expect_bool(false);
}

/// Array map empty.
#[test]
fn test_array_map_empty() {
    ShapeTest::new(r#"[].map(|x| x * 2).length"#).expect_number(0.0);
}

/// Array filter basic.
#[test]
fn test_array_filter_basic() {
    ShapeTest::new(r#"{ let a = [1, 2, 3, 4, 5]; a.filter(|x| x > 3).length }"#)
        .expect_number(2.0);
}

/// Array filter keep all.
#[test]
fn test_array_filter_keep_all() {
    ShapeTest::new(r#"[1, 2, 3].filter(|x| x > 0).length"#).expect_number(3.0);
}

/// Array filter keep none.
#[test]
fn test_array_filter_keep_none() {
    ShapeTest::new(r#"[1, 2, 3].filter(|x| x > 10).length"#).expect_number(0.0);
}

/// Array filter empty.
#[test]
fn test_array_filter_empty() {
    ShapeTest::new(r#"[].filter(|x| x > 0).length"#).expect_number(0.0);
}

/// Array reduce sum.
#[test]
fn test_array_reduce_sum() {
    ShapeTest::new(r#"[1, 2, 3, 4].reduce(|acc, x| acc + x, 0)"#).expect_number(10.0);
}

/// Array reduce product.
#[test]
fn test_array_reduce_product() {
    ShapeTest::new(r#"[1, 2, 3, 4].reduce(|acc, x| acc * x, 1)"#).expect_number(24.0);
}

/// Array reduce empty.
#[test]
fn test_array_reduce_empty() {
    ShapeTest::new(r#"[].reduce(|acc, x| acc + x, 42)"#).expect_number(42.0);
}

/// Array find found.
#[test]
fn test_array_find_found() {
    ShapeTest::new(r#"[10, 20, 30].find(|x| x > 15)"#).expect_number(20.0);
}

/// Array find not found.
#[test]
fn test_array_find_not_found() {
    ShapeTest::new(r#"[10, 20, 30].find(|x| x > 100)"#).expect_none();
}

/// Array some true.
#[test]
fn test_array_some_true() {
    ShapeTest::new(r#"[1, 2, 3].some(|x| x > 2)"#).expect_bool(true);
}

/// Array some false.
#[test]
fn test_array_some_false() {
    ShapeTest::new(r#"[1, 2, 3].some(|x| x > 10)"#).expect_bool(false);
}

/// Array every true.
#[test]
fn test_array_every_true() {
    ShapeTest::new(r#"[2, 4, 6].every(|x| x > 0)"#).expect_bool(true);
}

/// Array every false.
#[test]
fn test_array_every_false() {
    ShapeTest::new(r#"[2, 4, 6].every(|x| x > 3)"#).expect_bool(false);
}

/// Array filter then map — check sum.
#[test]
fn test_array_filter_then_map() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5, 6].filter(|x| x % 2 == 0).map(|x| x * 10).reduce(|acc, x| acc + x, 0)"#)
        .expect_number(120.0);
}

/// Array map then filter — check length.
#[test]
fn test_array_map_then_filter() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].map(|x| x * 2).filter(|x| x > 6).length"#)
        .expect_number(2.0);
}

/// Array filter map reduce.
#[test]
fn test_array_filter_map_reduce() {
    ShapeTest::new(
        r#"[1, 2, 3, 4, 5, 6].filter(|x| x % 2 == 0).map(|x| x * 10).reduce(|acc, x| acc + x, 0)"#,
    )
    .expect_number(120.0);
}

/// Array flatMap basic.
#[test]
fn test_array_flatmap_basic() {
    ShapeTest::new(r#"[[1, 2], [3, 4]].flatMap(|arr| arr).length"#).expect_number(4.0);
}

/// Array length after filter.
#[test]
fn test_array_length_after_filter() {
    ShapeTest::new(r#"{ let filtered = [1, 2, 3, 4, 5].filter(|x| x > 2); filtered.length }"#)
        .expect_number(3.0);
}

/// Array single element map.
#[test]
fn test_array_single_element_map() {
    ShapeTest::new(r#"[42].map(|x| x + 1)[0]"#).expect_number(43.0);
}

/// Array sum aggregation.
/// The builtin `sum()` expects Column/Number args, not Array method call.
/// Use `.reduce()` instead for array summation.
#[test]
fn test_array_sum_aggregation() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].reduce(|acc, x| acc + x, 0)"#).expect_number(15.0);
}

/// Array count aggregation.
#[test]
fn test_array_count_aggregation() {
    ShapeTest::new(r#"[10, 20, 30, 40].count()"#).expect_number(4.0);
}

// =============================================================================
// SECTION 2: Iterator .iter() Creation
// =============================================================================

/// Array iter collect identity.
#[test]
fn test_array_iter_collect_identity() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3].iter().collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(6.0);
}

/// Array iter toArray.
#[test]
fn test_array_iter_to_array() {
    ShapeTest::new(r#"[10, 20, 30].iter().toArray().length"#).expect_number(3.0);
}

/// Empty array iter collect.
#[test]
fn test_empty_array_iter_collect() {
    ShapeTest::new(r#"[].iter().collect().length"#).expect_number(0.0);
}

/// Single element array iter collect.
#[test]
fn test_single_element_array_iter_collect() {
    ShapeTest::new(r#"[99].iter().collect()[0]"#).expect_number(99.0);
}

/// Iter count.
#[test]
fn test_iter_count() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].iter().count()"#).expect_number(5.0);
}

/// Empty iter count.
#[test]
fn test_empty_iter_count() {
    ShapeTest::new(r#"[].iter().count()"#).expect_number(0.0);
}

// =============================================================================
// SECTION 3: Iterator Lazy take/skip
// =============================================================================

/// Iter take basic.
#[test]
fn test_iter_take_basic() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5].iter().take(3).collect()
            arr.length
        }
    "#).expect_number(3.0);
}

/// Iter take zero.
#[test]
fn test_iter_take_zero() {
    ShapeTest::new(r#"[1, 2, 3].iter().take(0).collect().length"#).expect_number(0.0);
}

/// Iter take more than available.
#[test]
fn test_iter_take_more_than_available() {
    ShapeTest::new(r#"[1, 2, 3].iter().take(10).collect().length"#).expect_number(3.0);
}

/// Iter take all.
#[test]
fn test_iter_take_all() {
    ShapeTest::new(r#"[1, 2, 3].iter().take(3).collect().length"#).expect_number(3.0);
}

/// Iter skip basic.
#[test]
fn test_iter_skip_basic() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5].iter().skip(2).collect()
            arr[0]
        }
    "#).expect_number(3.0);
}

/// Iter skip zero.
#[test]
fn test_iter_skip_zero() {
    ShapeTest::new(r#"[1, 2, 3].iter().skip(0).collect().length"#).expect_number(3.0);
}

/// Iter skip all.
#[test]
fn test_iter_skip_all() {
    ShapeTest::new(r#"[1, 2, 3].iter().skip(3).collect().length"#).expect_number(0.0);
}

/// Iter skip more than available.
#[test]
fn test_iter_skip_more_than_available() {
    ShapeTest::new(r#"[1, 2, 3].iter().skip(10).collect().length"#).expect_number(0.0);
}

/// Iter skip then take.
#[test]
fn test_iter_skip_then_take() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5].iter().skip(1).take(3).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(9.0);
}

/// Iter take then skip.
#[test]
fn test_iter_take_then_skip() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5].iter().take(4).skip(2).collect()
            arr[0] + arr[1]
        }
    "#).expect_number(7.0);
}

/// Iter skip then take then count.
#[test]
fn test_iter_skip_then_take_then_count() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].iter().skip(2).take(5).count()"#)
        .expect_number(5.0);
}

// =============================================================================
// SECTION 4: Iterator Lazy map/filter (with closures)
// =============================================================================

/// Iter map collect.
#[test]
fn test_iter_map_collect() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3].iter().map(|x| x * 10).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(60.0);
}

/// Iter map identity.
#[test]
fn test_iter_map_identity() {
    ShapeTest::new(r#"[5, 10, 15].iter().map(|x| x).collect()[0]"#).expect_number(5.0);
}

/// Iter map empty.
#[test]
fn test_iter_map_empty() {
    ShapeTest::new(r#"[].iter().map(|x| x * 2).collect().length"#).expect_number(0.0);
}

/// Iter filter collect.
#[test]
fn test_iter_filter_collect() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5].iter().filter(|x| x > 3).collect()
            arr[0] + arr[1]
        }
    "#).expect_number(9.0);
}

/// Iter filter keep all.
#[test]
fn test_iter_filter_keep_all() {
    ShapeTest::new(r#"[1, 2, 3].iter().filter(|x| x > 0).collect().length"#)
        .expect_number(3.0);
}

/// Iter filter keep none.
#[test]
fn test_iter_filter_keep_none() {
    ShapeTest::new(r#"[1, 2, 3].iter().filter(|x| x > 100).collect().length"#)
        .expect_number(0.0);
}

/// Iter filter empty source.
#[test]
fn test_iter_filter_empty_source() {
    ShapeTest::new(r#"[].iter().filter(|x| x > 0).collect().length"#).expect_number(0.0);
}

/// Iter filter even numbers.
#[test]
fn test_iter_filter_even_numbers() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5, 6].iter().filter(|x| x % 2 == 0).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(12.0);
}
