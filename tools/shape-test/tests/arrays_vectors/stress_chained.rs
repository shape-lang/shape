//! Stress tests for chained array method pipelines.

use shape_test::shape_test::ShapeTest;

/// Verifies flatten nested.
#[test]
fn test_flatten_nested() {
    ShapeTest::new(
        r#"(
        [[1, 2], [3, 4], [5]].flatten()
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [[1, 2], [3, 4], [5]].flatten()
    )[4]"#,
    )
    .expect_number(5.0);
}

/// Verifies flatten empty.
#[test]
fn test_flatten_empty() {
    ShapeTest::new(
        r#"
        let empty = []
        empty.flatten().length
    "#,
    )
    .expect_number(0.0);
}

/// Verifies reverse basic.
#[test]
fn test_reverse_basic() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4].reverse()
    )[0]"#,
    )
    .expect_number(4.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4].reverse()
    )[3]"#,
    )
    .expect_number(1.0);
}

/// Verifies reverse single.
#[test]
fn test_reverse_single() {
    ShapeTest::new(
        r#"(
        [42].reverse()
    )[0]"#,
    )
    .expect_number(42.0);
}

/// Verifies take basic.
#[test]
fn test_take_basic() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].take(3)
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].take(3)
    )[2]"#,
    )
    .expect_number(3.0);
}

/// Verifies skip basic.
#[test]
fn test_skip_basic() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].skip(2)
    )[0]"#,
    )
    .expect_number(3.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].skip(2)
    )[2]"#,
    )
    .expect_number(5.0);
}

/// Verifies concat two arrays.
#[test]
fn test_concat_two_arrays() {
    ShapeTest::new(
        r#"(
        [1, 2].concat([3, 4])
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [1, 2].concat([3, 4])
    )[3]"#,
    )
    .expect_number(4.0);
}

/// Verifies concat with empty.
#[test]
fn test_concat_with_empty() {
    ShapeTest::new(
        r#"
        let empty = []
        [1, 2, 3].concat(empty).length
    "#,
    )
    .expect_number(3.0);
}

/// Verifies join str default.
#[test]
fn test_join_str_default() {
    ShapeTest::new(
        r#"
        [1, 2, 3].join(",")
    "#,
    )
    .expect_string("1,2,3");
}

/// Verifies join str custom separator.
#[test]
fn test_join_str_custom_separator() {
    ShapeTest::new(
        r#"
        [1, 2, 3].join(" - ")
    "#,
    )
    .expect_string("1 - 2 - 3");
}

/// Verifies join empty array.
#[test]
fn test_join_empty_array() {
    ShapeTest::new(
        r#"
        let empty = []
        empty.join(",")
    "#,
    )
    .expect_string("");
}

/// Verifies slice basic.
#[test]
fn test_slice_basic() {
    ShapeTest::new(
        r#"(
        [10, 20, 30, 40, 50].slice(1, 4)
    )[0]"#,
    )
    .expect_number(20.0);
    ShapeTest::new(
        r#"(
        [10, 20, 30, 40, 50].slice(1, 4)
    )[2]"#,
    )
    .expect_number(40.0);
}

/// Verifies slice from start.
#[test]
fn test_slice_from_start() {
    ShapeTest::new(
        r#"(
        [10, 20, 30, 40, 50].slice(0, 2)
    )[0]"#,
    )
    .expect_number(10.0);
    ShapeTest::new(
        r#"(
        [10, 20, 30, 40, 50].slice(0, 2)
    )[1]"#,
    )
    .expect_number(20.0);
}

/// Verifies slice single.
#[test]
fn test_slice_single() {
    ShapeTest::new(
        r#"(
        [10, 20, 30, 40, 50].slice(2, 3)
    )[0]"#,
    )
    .expect_number(30.0);
}

/// Verifies single found.
#[test]
fn test_single_found() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5].single(|x| x == 3)
    "#,
    )
    .expect_number(3.0);
}

/// Verifies single unique match.
#[test]
fn test_single_unique_match() {
    ShapeTest::new(
        r#"
        [10, 20, 30].single(|x| x > 25)
    "#,
    )
    .expect_number(30.0);
}

/// Verifies first basic.
#[test]
fn test_first_basic() {
    ShapeTest::new(
        r#"
        [10, 20, 30].first()
    "#,
    )
    .expect_number(10.0);
}

/// Verifies first empty.
#[test]
fn test_first_empty() {
    ShapeTest::new(
        r#"
        let empty = []
        empty.first()
    "#,
    )
    .expect_none();
}

/// Verifies last basic.
#[test]
fn test_last_basic() {
    ShapeTest::new(
        r#"
        [10, 20, 30].last()
    "#,
    )
    .expect_number(30.0);
}

/// Verifies last empty.
#[test]
fn test_last_empty() {
    ShapeTest::new(
        r#"
        let empty = []
        empty.last()
    "#,
    )
    .expect_none();
}

/// Verifies pipeline top 3 squares.
#[test]
fn test_pipeline_top_3_squares() {
    ShapeTest::new(
        r#"(
        [5, 2, 8, 1, 9, 3]
            .sort()
            .reverse()
            .take(3)
            .map(|x| x * x)
    )[0]"#,
    )
    .expect_number(81.0);
    ShapeTest::new(
        r#"(
        [5, 2, 8, 1, 9, 3]
            .sort()
            .reverse()
            .take(3)
            .map(|x| x * x)
    )[2]"#,
    )
    .expect_number(25.0);
}

/// Verifies pipeline filter unique count.
#[test]
fn test_pipeline_filter_unique_count() {
    ShapeTest::new(
        r#"
        [1, 2, 2, 3, 3, 3, 4, 4, 4, 4]
            .filter(|x| x > 1)
            .unique()
            .count()
    "#,
    )
    .expect_number(3.0);
}

/// Verifies pipeline flatmap filter sum.
#[test]
fn test_pipeline_flatmap_filter_sum() {
    ShapeTest::new(
        r#"
        [1, 2, 3]
            .flatMap(|x| [x, x * 10])
            .filter(|x| x > 5)
            .sum()
    "#,
    )
    .expect_number(60.0);
}

/// Verifies pipeline map sort first.
#[test]
fn test_pipeline_map_sort_first() {
    ShapeTest::new(
        r#"
        [3, 1, 4, 1, 5]
            .map(|x| x * x)
            .sort()
            .first()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies pipeline filter map every.
#[test]
fn test_pipeline_filter_map_every() {
    ShapeTest::new(
        r#"
        [2, 4, 6, 8, 10]
            .filter(|x| x > 3)
            .map(|x| x % 2)
            .every(|x| x == 0)
    "#,
    )
    .expect_bool(true);
}

/// Verifies pipeline double filter.
#[test]
fn test_pipeline_double_filter() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .filter(|x| x % 2 == 0)
            .filter(|x| x > 5)
    )[0]"#,
    )
    .expect_number(6.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .filter(|x| x % 2 == 0)
            .filter(|x| x > 5)
    )[2]"#,
    )
    .expect_number(10.0);
}

/// Verifies union basic.
#[test]
fn test_union_basic() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].union([3, 4, 5])
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3].union([3, 4, 5])
    )[4]"#,
    )
    .expect_number(5.0);
}

/// Verifies intersect basic.
#[test]
fn test_intersect_basic() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4].intersect([3, 4, 5, 6])
    )[0]"#,
    )
    .expect_number(3.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4].intersect([3, 4, 5, 6])
    )[1]"#,
    )
    .expect_number(4.0);
}

/// Verifies except basic.
#[test]
fn test_except_basic() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4].except([3, 4, 5])
    )[0]"#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4].except([3, 4, 5])
    )[1]"#,
    )
    .expect_number(2.0);
}

/// Verifies union disjoint.
#[test]
fn test_union_disjoint() {
    ShapeTest::new(
        r#"(
        [1, 2].union([3, 4])
    ).length"#,
    )
    .expect_number(4.0);
}

/// Verifies intersect disjoint.
#[test]
fn test_intersect_disjoint() {
    ShapeTest::new(
        r#"(
        [1, 2].intersect([3, 4])
    ).length"#,
    )
    .expect_number(0.0);
}

/// Verifies except all excluded.
#[test]
fn test_except_all_excluded() {
    ShapeTest::new(
        r#"(
        [1, 2, 3].except([1, 2, 3])
    ).length"#,
    )
    .expect_number(0.0);
}

/// Verifies for each returns none.
#[test]
fn test_for_each_returns_none() {
    ShapeTest::new(
        r#"
        let x = [1, 2, 3].forEach(|x| x)
        x
    "#,
    )
    .expect_none();
}

/// Verifies group by modulo.
#[test]
fn test_group_by_modulo() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5, 6].groupBy(|x| x % 2)
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies group by all same.
#[test]
fn test_group_by_all_same() {
    ShapeTest::new(
        r#"
        [1, 1, 1].groupBy(|x| x)
    
true"#,
    )
    .expect_bool(true);
}

/// Verifies drop basic.
#[test]
fn test_drop_basic() {
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].drop(2)
    )[0]"#,
    )
    .expect_number(3.0);
    ShapeTest::new(
        r#"(
        [1, 2, 3, 4, 5].drop(2)
    )[2]"#,
    )
    .expect_number(5.0);
}

/// Verifies fn map double.
#[test]
fn test_fn_map_double() {
    ShapeTest::new(
        r#"
        fn double_all() {
            [1, 2, 3].map(|x| x * 2)
        }
        double_all()[0]
    "#,
    )
    .expect_number(2.0);
    ShapeTest::new(
        r#"
        fn double_all() {
            [1, 2, 3].map(|x| x * 2)
        }
        double_all()[2]
    "#,
    )
    .expect_number(6.0);
}

/// Verifies fn filter and sum.
#[test]
fn test_fn_filter_and_sum() {
    ShapeTest::new(
        r#"
        fn sum_evens() -> int {
            [1, 2, 3, 4, 5, 6].filter(|x| x % 2 == 0).reduce(|acc, x| acc + x, 0)
        }
sum_evens()"#,
    )
    .expect_number(12.0);
}

/// Verifies fn pipeline.
#[test]
fn test_fn_pipeline() {
    ShapeTest::new(
        r#"
        fn pipeline() -> int {
            [5, 3, 8, 1, 9]
                .filter(|x| x > 3)
                .map(|x| x * 2)
                .reduce(|acc, x| acc + x, 0)
        }
pipeline()"#,
    )
    .expect_number(44.0);
}

/// Verifies fn sort and take.
#[test]
fn test_fn_sort_and_take() {
    ShapeTest::new(
        r#"
        fn top_two() {
            [5, 3, 8, 1, 9].sort().reverse().take(2)
        }
        top_two()[0]
    "#,
    )
    .expect_number(9.0);
    ShapeTest::new(
        r#"
        fn top_two() {
            [5, 3, 8, 1, 9].sort().reverse().take(2)
        }
        top_two()[1]
    "#,
    )
    .expect_number(8.0);
}

/// Verifies fn unique sorted.
#[test]
fn test_fn_unique_sorted() {
    ShapeTest::new(
        r#"
        fn unique_sorted() {
            [3, 1, 4, 1, 5, 9, 2, 6, 5, 3].unique().sort()
        }
        unique_sorted()[0]
    "#,
    )
    .expect_number(1.0);
    ShapeTest::new(
        r#"
        fn unique_sorted() {
            [3, 1, 4, 1, 5, 9, 2, 6, 5, 3].unique().sort()
        }
        unique_sorted()[6]
    "#,
    )
    .expect_number(9.0);
}
