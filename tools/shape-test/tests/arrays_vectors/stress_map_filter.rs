//! Stress tests for array map and filter operations.

use shape_test::shape_test::ShapeTest;

/// Verifies map identity.
#[test]
fn test_map_identity() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| x)
    ).length"#,
    )
    .expect_number(3.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| x)
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| x)
    )[2]"#,
    )
    .expect_number(3.0);
}

/// Verifies map double.
#[test]
fn test_map_double() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| x * 2)
    )[0]"#,
    )
    .expect_number(2.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| x * 2)
    )[2]"#,
    )
    .expect_number(6.0);
}

/// Verifies map to float.
#[test]
fn test_map_to_float() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| x * 1.5)
    )[0]"#,
    )
    .expect_number(1.5);
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| x * 1.5)
    )[2]"#,
    )
    .expect_number(4.5);
}

/// Verifies map empty array.
#[test]
fn test_map_empty_array() {
    ShapeTest::new(
        r#"
        let empty = []
        empty.map(|x| x * 2).length
    "#,
    )
    .expect_number(0.0);
}

/// Verifies map single element.
#[test]
fn test_map_single_element() {
    ShapeTest::new(
        r#"(
        [42].map(|x| x + 1)
    )[0]"#,
    )
    .expect_number(43.0);
}

/// Verifies map to bool.
#[test]
fn test_map_to_bool() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| x > 1)
    )[0]"#,
    )
    .expect_bool(false);
    ShapeTest::new(
        r#"(
        [1, 2, 3].map(|x| x > 1)
    )[2]"#,
    )
    .expect_bool(true);
}

/// Verifies map negate.
#[test]
fn test_map_negate() {
    ShapeTest::new(
        r#"(
        [1, -2, 3].map(|x| -x)
    )[0]"#,
    )
    .expect_number(-1.0);
    ShapeTest::new(
        r#"(
        [1, -2, 3].map(|x| -x)
    )[2]"#,
    )
    .expect_number(-3.0);
}

/// Verifies map with index.
#[test]
fn test_map_with_index() {
    ShapeTest::new(
        r#"(
        [10, 20, 30].map(|x, i| x + i)
    )[0]"#,
    )
    .expect_number(10.0);
    ShapeTest::new(
        r#"(
        [10, 20, 30].map(|x, i| x + i)
    )[2]"#,
    )
    .expect_number(32.0);
}

/// Verifies map squared.
#[test]
fn test_map_squared() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].map(|x| x * x)
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].map(|x| x * x)
    )[4]"#,
    )
    .expect_number(25.0);
}

/// Verifies map add constant.
#[test]
fn test_map_add_constant() {
    ShapeTest::new(
        r#"(
        [0, 0, 0].map(|x| x + 100)
    )[0]"#,
    )
    .expect_number(100.0);
    ShapeTest::new(
        r#"(
        [0, 0, 0].map(|x| x + 100)
    )[2]"#,
    )
    .expect_number(100.0);
}

/// Verifies filter all match.
#[test]
fn test_filter_all_match() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].filter(|x| x > 0)
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies filter none match.
#[test]
fn test_filter_none_match() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].filter(|x| x > 10)
    ).length"#,
    )
    .expect_number(0.0);
}

/// Verifies filter some match.
#[test]
fn test_filter_some_match() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].filter(|x| x > 3)
    )[0]"#,
    )
    .expect_number(4.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].filter(|x| x > 3)
    )[1]"#,
    )
    .expect_number(5.0);
}

/// Verifies filter empty array.
#[test]
fn test_filter_empty_array() {
    ShapeTest::new(
        r#"
        let empty = []
        empty.filter(|x| x > 0).length
    "#,
    )
    .expect_number(0.0);
}

/// Verifies filter single element pass.
#[test]
fn test_filter_single_element_pass() {
    ShapeTest::new(
        r#"(
        [5].filter(|x| x > 3)
    ).length"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [5].filter(|x| x > 3)
    )[0]"#,
    )
    .expect_number(5.0);
}

/// Verifies filter single element fail.
#[test]
fn test_filter_single_element_fail() {
    ShapeTest::new(
        r#"(
        [1].filter(|x| x > 3)
    ).length"#,
    )
    .expect_number(0.0);
}

/// Verifies filter even.
#[test]
fn test_filter_even() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5, 6].filter(|x| x % 2 == 0)
    )[0]"#,
    )
    .expect_number(2.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5, 6].filter(|x| x % 2 == 0)
    )[2]"#,
    )
    .expect_number(6.0);
}

/// Verifies filter odd.
#[test]
fn test_filter_odd() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5, 6].filter(|x| x % 2 != 0)
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5, 6].filter(|x| x % 2 != 0)
    )[2]"#,
    )
    .expect_number(5.0);
}

/// Verifies filter negative.
#[test]
fn test_filter_negative() {
    ShapeTest::new(
        r#"(
        [-3, -2, -1, 0, 1, 2, 3].filter(|x| x < 0)
    )[0]"#,
    )
    .expect_number(-3.0);
    ShapeTest::new(
        r#"(
        [-3, -2, -1, 0, 1, 2, 3].filter(|x| x < 0)
    )[2]"#,
    )
    .expect_number(-1.0);
}

/// Verifies filter with index.
#[test]
fn test_filter_with_index() {
    ShapeTest::new(
        r#"(
        [10, 20, 30, 40, 50].filter(|x, i| i >= 2)
    )[0]"#,
    )
    .expect_number(30.0);
    ShapeTest::new(
        r#"(
        [10, 20, 30, 40, 50].filter(|x, i| i >= 2)
    )[2]"#,
    )
    .expect_number(50.0);
}

/// Verifies reduce sum.
#[test]
fn test_reduce_sum() {
    ShapeTest::new(
        r#"
        [1, 2, 3].reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(6.0);
}

/// Verifies reduce product.
#[test]
fn test_reduce_product() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4].reduce(|acc, x| acc * x, 1)
    "#,
    )
    .expect_number(24.0);
}

/// Verifies reduce single element.
#[test]
fn test_reduce_single_element() {
    ShapeTest::new(
        r#"
        [42].reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies reduce empty returns initial.
#[test]
fn test_reduce_empty_returns_initial() {
    ShapeTest::new(
        r#"
        let empty = []
        empty.reduce(|acc, x| acc + x, 99)
    "#,
    )
    .expect_number(99.0);
}

/// Verifies reduce subtract.
#[test]
fn test_reduce_subtract() {
    ShapeTest::new(
        r#"
        [1, 2, 3].reduce(|acc, x| acc - x, 10)
    "#,
    )
    .expect_number(4.0);
}

/// Verifies reduce max manual.
#[test]
fn test_reduce_max_manual() {
    ShapeTest::new(
        r#"
        fn my_max(a: int, b: int) -> int {
            if a > b { a } else { b }
        }
        [3, 7, 2, 9, 1].reduce(|acc, x| my_max(acc, x), 0)
    "#,
    )
    .expect_number(9.0);
}

/// Verifies reduce with float initial.
#[test]
fn test_reduce_with_float_initial() {
    ShapeTest::new(
        r#"
        [1.0, 2.0, 3.0].reduce(|acc, x| acc + x, 0.5)
    "#,
    )
    .expect_number(6.5);
}

/// Verifies reduce count positive.
#[test]
fn test_reduce_count_positive() {
    ShapeTest::new(
        r#"
        [-1, 2, -3, 4, -5].reduce(|acc, x| if x > 0 { acc + 1 } else { acc }, 0)
    "#,
    )
    .expect_number(2.0);
}

/// Verifies sort natural.
#[test]
fn test_sort_natural() {
    ShapeTest::new(
        r#"(
        [3, 1, 2].sort()
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [3, 1, 2].sort()
    )[2]"#,
    )
    .expect_number(3.0);
}

/// Verifies sort already sorted.
#[test]
fn test_sort_already_sorted() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].sort()
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3].sort()
    )[2]"#,
    )
    .expect_number(3.0);
}

/// Verifies sort reverse sorted.
#[test]
fn test_sort_reverse_sorted() {
    ShapeTest::new(
        r#"(
        [5, 4, 3, 2, 1].sort()
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [5, 4, 3, 2, 1].sort()
    )[4]"#,
    )
    .expect_number(5.0);
}

/// Verifies sort single element.
#[test]
fn test_sort_single_element() {
    ShapeTest::new(
        r#"(
        [42].sort()
    )[0]"#,
    )
    .expect_number(42.0);
}

/// Verifies sort empty.
#[test]
fn test_sort_empty() {
    ShapeTest::new(
        r#"
        let empty = []
        empty.sort().length
    "#,
    )
    .expect_number(0.0);
}

/// Verifies sort with comparator.
#[test]
fn test_sort_with_comparator() {
    ShapeTest::new(
        r#"(
        [3, 1, 2].sort(|a, b| a - b)
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [3, 1, 2].sort(|a, b| a - b)
    )[2]"#,
    )
    .expect_number(3.0);
}

/// Verifies sort descending comparator.
#[test]
fn test_sort_descending_comparator() {
    ShapeTest::new(
        r#"(
        [3, 1, 2].sort(|a, b| b - a)
    )[0]"#,
    )
    .expect_number(3.0);
    ShapeTest::new(
        r#"(
        [3, 1, 2].sort(|a, b| b - a)
    )[2]"#,
    )
    .expect_number(1.0);
}

/// Verifies sort duplicates.
#[test]
fn test_sort_duplicates() {
    ShapeTest::new(
        r#"(
        [3, 1, 3, 2, 1].sort()
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [3, 1, 3, 2, 1].sort()
    )[4]"#,
    )
    .expect_number(3.0);
}

/// Verifies sort negative values.
#[test]
fn test_sort_negative_values() {
    ShapeTest::new(
        r#"(
        [3, -1, 0, -5, 2].sort()
    )[0]"#,
    )
    .expect_number(-5.0);
    ShapeTest::new(
        r#"(
        [3, -1, 0, -5, 2].sort()
    )[4]"#,
    )
    .expect_number(3.0);
}

/// Verifies unique with duplicates.
#[test]
fn test_unique_with_duplicates() {
    ShapeTest::new(
        r#"(
        [1, 2, 2, 3, 3, 3].unique()
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [1, 2, 2, 3, 3, 3].unique()
    )[2]"#,
    )
    .expect_number(3.0);
}

/// Verifies unique all unique.
#[test]
fn test_unique_all_unique() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].unique()
    ).length"#,
    )
    .expect_number(3.0);
}

/// Verifies unique all same.
#[test]
fn test_unique_all_same() {
    ShapeTest::new(
        r#"(
        [5, 5, 5, 5].unique()
    )[0]"#,
    )
    .expect_number(5.0);
}

/// Verifies unique empty.
#[test]
fn test_unique_empty() {
    ShapeTest::new(
        r#"
        let empty = []
        empty.unique().length
    "#,
    )
    .expect_number(0.0);
}

/// Verifies unique single.
#[test]
fn test_unique_single() {
    ShapeTest::new(
        r#"(
        [42].unique()
    )[0]"#,
    )
    .expect_number(42.0);
}

/// Verifies unique preserves order.
#[test]
fn test_unique_preserves_order() {
    ShapeTest::new(
        r#"(
        [3, 1, 2, 1, 3].unique()
    )[0]"#,
    )
    .expect_number(3.0);
    ShapeTest::new(
        r#"(
        [3, 1, 2, 1, 3].unique()
    )[2]"#,
    )
    .expect_number(2.0);
}
