//! Stress tests for array sort, find, and set operations.

use shape_test::shape_test::ShapeTest;


/// Verifies max single.
#[test]
fn test_max_single() {
    ShapeTest::new(r#"
        [99].max()
    "#)
    .expect_number(99.0);
}

/// Verifies where basic.
#[test]
fn test_where_basic() {
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5].where(|x| x > 3)
    )[0]"#)
    .expect_number(4.0);
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5].where(|x| x > 3)
    )[1]"#)
    .expect_number(5.0);
}

/// Verifies where none match.
#[test]
fn test_where_none_match() {
    ShapeTest::new(r#"(
        [1, 2, 3].where(|x| x > 10)
    ).length"#)
    .expect_number(0.0);
}

/// Verifies where all match.
#[test]
fn test_where_all_match() {
    ShapeTest::new(r#"(
        [10, 20, 30].where(|x| x > 0)
    ).length"#)
    .expect_number(3.0);
}

/// Verifies select double.
#[test]
fn test_select_double() {
    ShapeTest::new(r#"(
        [1, 2, 3].select(|x| x * 2)
    )[0]"#)
    .expect_number(2.0);
    ShapeTest::new(r#"(
        [1, 2, 3].select(|x| x * 2)
    )[2]"#)
    .expect_number(6.0);
}

/// Verifies select empty.
#[test]
fn test_select_empty() {
    ShapeTest::new(r#"
        let empty = []
        empty.select(|x| x + 1).length
    "#)
    .expect_number(0.0);
}

/// Verifies select identity.
#[test]
fn test_select_identity() {
    ShapeTest::new(r#"(
        [5, 10, 15].select(|x| x)
    )[0]"#)
    .expect_number(5.0);
    ShapeTest::new(r#"(
        [5, 10, 15].select(|x| x)
    )[2]"#)
    .expect_number(15.0);
}

/// Verifies order by identity.
#[test]
fn test_order_by_identity() {
    ShapeTest::new(r#"(
        [3, 1, 2].orderBy(|x| x)
    )[0]"#)
    .expect_number(1.0);
    ShapeTest::new(r#"(
        [3, 1, 2].orderBy(|x| x)
    )[2]"#)
    .expect_number(3.0);
}

/// Verifies order by descending.
#[test]
fn test_order_by_descending() {
    ShapeTest::new(r#"(
        [3, 1, 2].orderBy(|x| x, "desc")
    )[0]"#)
    .expect_number(3.0);
    ShapeTest::new(r#"(
        [3, 1, 2].orderBy(|x| x, "desc")
    )[2]"#)
    .expect_number(1.0);
}

/// Verifies order by negative key.
#[test]
fn test_order_by_negative_key() {
    ShapeTest::new(r#"(
        [3, 1, 2].orderBy(|x| -x)
    )[0]"#)
    .expect_number(3.0);
    ShapeTest::new(r#"(
        [3, 1, 2].orderBy(|x| -x)
    )[2]"#)
    .expect_number(1.0);
}

/// Verifies order by empty.
#[test]
fn test_order_by_empty() {
    ShapeTest::new(r#"
        let empty = []
        empty.orderBy(|x| x).length
    "#)
    .expect_number(0.0);
}

/// Verifies take while basic.
#[test]
fn test_take_while_basic() {
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5].takeWhile(|x| x < 4)
    )[0]"#)
    .expect_number(1.0);
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5].takeWhile(|x| x < 4)
    )[2]"#)
    .expect_number(3.0);
}

/// Verifies take while all.
#[test]
fn test_take_while_all() {
    ShapeTest::new(r#"(
        [1, 2, 3].takeWhile(|x| x < 10)
    ).length"#)
    .expect_number(3.0);
}

/// Verifies take while none.
#[test]
fn test_take_while_none() {
    ShapeTest::new(r#"(
        [5, 6, 7].takeWhile(|x| x < 1)
    ).length"#)
    .expect_number(0.0);
}

/// Verifies skip while basic.
#[test]
fn test_skip_while_basic() {
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5].skipWhile(|x| x < 3)
    )[0]"#)
    .expect_number(3.0);
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5].skipWhile(|x| x < 3)
    )[2]"#)
    .expect_number(5.0);
}

/// Verifies skip while all.
#[test]
fn test_skip_while_all() {
    ShapeTest::new(r#"(
        [1, 2, 3].skipWhile(|x| x < 10)
    ).length"#)
    .expect_number(0.0);
}

/// Verifies skip while none.
#[test]
fn test_skip_while_none() {
    ShapeTest::new(r#"(
        [5, 6, 7].skipWhile(|x| x < 1)
    ).length"#)
    .expect_number(3.0);
}

/// Verifies chain map then filter.
#[test]
fn test_chain_map_then_filter() {
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5].map(|x| x * 2).filter(|x| x > 6)
    )[0]"#)
    .expect_number(8.0);
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5].map(|x| x * 2).filter(|x| x > 6)
    )[1]"#)
    .expect_number(10.0);
}

/// Verifies chain filter then map.
#[test]
fn test_chain_filter_then_map() {
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5].filter(|x| x > 2).map(|x| x * 10)
    )[0]"#)
    .expect_number(30.0);
    ShapeTest::new(r#"(
        [1, 2, 3, 4, 5].filter(|x| x > 2).map(|x| x * 10)
    )[2]"#)
    .expect_number(50.0);
}

/// Verifies chain filter then sum.
#[test]
fn test_chain_filter_then_sum() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5].filter(|x| x > 2).sum()
    "#)
    .expect_number(12.0);
}

/// Verifies chain map then sum.
#[test]
fn test_chain_map_then_sum() {
    ShapeTest::new(r#"
        [1, 2, 3].map(|x| x * 2).sum()
    "#)
    .expect_number(12.0);
}

/// Verifies chain filter then sort.
#[test]
fn test_chain_filter_then_sort() {
    ShapeTest::new(r#"(
        [5, 3, 4, 1, 2].filter(|x| x > 2).sort()
    )[0]"#)
    .expect_number(3.0);
    ShapeTest::new(r#"(
        [5, 3, 4, 1, 2].filter(|x| x > 2).sort()
    )[2]"#)
    .expect_number(5.0);
}

/// Verifies chain sort then take.
#[test]
fn test_chain_sort_then_take() {
    ShapeTest::new(r#"(
        [5, 3, 4, 1, 2].sort().take(3)
    )[0]"#)
    .expect_number(1.0);
    ShapeTest::new(r#"(
        [5, 3, 4, 1, 2].sort().take(3)
    )[2]"#)
    .expect_number(3.0);
}

/// Verifies chain unique then sort.
#[test]
fn test_chain_unique_then_sort() {
    ShapeTest::new(r#"(
        [3, 1, 2, 1, 3].unique().sort()
    )[0]"#)
    .expect_number(1.0);
    ShapeTest::new(r#"(
        [3, 1, 2, 1, 3].unique().sort()
    )[2]"#)
    .expect_number(3.0);
}

/// Verifies chain filter map reduce.
#[test]
fn test_chain_filter_map_reduce() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5]
            .filter(|x| x > 1)
            .map(|x| x * 2)
            .reduce(|acc, x| acc + x, 0)
    "#)
    .expect_number(28.0);
}

/// Verifies chain filter map sum.
#[test]
fn test_chain_filter_map_sum() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5]
            .filter(|x| x % 2 != 0)
            .map(|x| x * x)
            .sum()
    "#)
    .expect_number(35.0);
}

/// Verifies chain map filter sort.
#[test]
fn test_chain_map_filter_sort() {
    ShapeTest::new(r#"(
        [5, 2, 8, 1, 9]
            .map(|x| x * 2)
            .filter(|x| x > 5)
            .sort()
    )[0]"#)
    .expect_number(10.0);
    ShapeTest::new(r#"(
        [5, 2, 8, 1, 9]
            .map(|x| x * 2)
            .filter(|x| x > 5)
            .sort()
    )[2]"#)
    .expect_number(18.0);
}

/// Verifies chain filter unique sort.
#[test]
fn test_chain_filter_unique_sort() {
    ShapeTest::new(r#"(
        [3, 1, 2, 3, 1, 2, 4]
            .filter(|x| x > 1)
            .unique()
            .sort()
    )[0]"#)
    .expect_number(2.0);
    ShapeTest::new(r#"(
        [3, 1, 2, 3, 1, 2, 4]
            .filter(|x| x > 1)
            .unique()
            .sort()
    )[2]"#)
    .expect_number(4.0);
}

/// Verifies chain sort reverse take.
#[test]
fn test_chain_sort_reverse_take() {
    ShapeTest::new(r#"(
        [5, 3, 8, 1, 9]
            .sort()
            .reverse()
            .take(3)
    )[0]"#)
    .expect_number(9.0);
    ShapeTest::new(r#"(
        [5, 3, 8, 1, 9]
            .sort()
            .reverse()
            .take(3)
    )[2]"#)
    .expect_number(5.0);
}

/// Verifies chain four ops.
#[test]
fn test_chain_four_ops() {
    ShapeTest::new(r#"(
        [10, 3, 7, 1, 5, 9, 2]
            .filter(|x| x > 3)
            .map(|x| x - 1)
            .sort()
            .reverse()
    )[0]"#)
    .expect_number(9.0);
    ShapeTest::new(r#"(
        [10, 3, 7, 1, 5, 9, 2]
            .filter(|x| x > 3)
            .map(|x| x - 1)
            .sort()
            .reverse()
    )[3]"#)
    .expect_number(4.0);
}

/// Verifies empty through map filter.
#[test]
fn test_empty_through_map_filter() {
    ShapeTest::new(r#"
        let empty = []
        empty.map(|x| x * 2).filter(|x| x > 0).length
    "#)
    .expect_number(0.0);
}

/// Verifies empty through sort unique.
#[test]
fn test_empty_through_sort_unique() {
    ShapeTest::new(r#"
        let empty = []
        empty.sort().unique().length
    "#)
    .expect_number(0.0);
}

/// Verifies filter to empty then reduce.
#[test]
fn test_filter_to_empty_then_reduce() {
    ShapeTest::new(r#"
        [1, 2, 3].filter(|x| x > 10).reduce(|acc, x| acc + x, 0)
    "#)
    .expect_number(0.0);
}

/// Verifies large array map.
#[test]
fn test_large_array_map() {
    ShapeTest::new(r#"
        fn make_array() {
            var arr = []
            var i = 0
            while i < 100 {
                arr = arr.concat([i])
                i = i + 1
            }
            arr
        }
        make_array().map(|x| x * 2).sum()
    "#)
    .expect_number(9900.0);
}

/// Verifies large array filter.
#[test]
fn test_large_array_filter() {
    ShapeTest::new(r#"
        fn make_array() {
            let arr = []
            let i = 0
            while i < 100 {
                arr = arr.concat([i])
                i = i + 1
            }
            arr
        }
        make_array().filter(|x| x >= 50).count()
    "#)
    .expect_number(50.0);
}

/// Verifies large array sort reverse.
#[test]
fn test_large_array_sort_reverse() {
    ShapeTest::new(r#"
        fn make_rev_array() {
            let arr = []
            let i = 20
            while i > 0 {
                arr = arr.concat([i])
                i = i - 1
            }
            arr
        }
        let sorted = make_rev_array().sort()
        sorted.first()
    "#)
    .expect_number(1.0);
}

/// Verifies large array unique.
#[test]
fn test_large_array_unique() {
    ShapeTest::new(r#"
        fn make_dup_array() {
            let arr = []
            let i = 0
            while i < 50 {
                arr = arr.concat([i % 10])
                i = i + 1
            }
            arr
        }
        make_dup_array().unique().sort()[0]
    "#)
    .expect_number(0.0);
    ShapeTest::new(r#"
        fn make_dup_array() {
            let arr = []
            let i = 0
            while i < 50 {
                arr = arr.concat([i % 10])
                i = i + 1
            }
            arr
        }
        make_dup_array().unique().sort()[9]
    "#)
    .expect_number(9.0);
}

/// Verifies large array reduce.
#[test]
fn test_large_array_reduce() {
    ShapeTest::new(r#"
        fn make_array() {
            let arr = []
            let i = 1
            while i <= 50 {
                arr = arr.concat([i])
                i = i + 1
            }
            arr
        }
        make_array().reduce(|acc, x| acc + x, 0)
    "#)
    .expect_number(1275.0);
}

/// Verifies large pipeline.
#[test]
fn test_large_pipeline() {
    ShapeTest::new(r#"
        fn make_array() {
            let arr = []
            let i = 0
            while i < 100 {
                arr = arr.concat([i])
                i = i + 1
            }
            arr
        }
        make_array()
            .filter(|x| x % 3 == 0)
            .map(|x| x * x)
            .reduce(|acc, x| acc + x, 0)
    "#)
    .expect_number(112761.0);
}

/// Verifies includes found.
#[test]
fn test_includes_found() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5].includes(3)
    "#)
    .expect_bool(true);
}

/// Verifies includes not found.
#[test]
fn test_includes_not_found() {
    ShapeTest::new(r#"
        [1, 2, 3].includes(99)
    "#)
    .expect_bool(false);
}

/// Verifies index of found.
#[test]
fn test_index_of_found() {
    ShapeTest::new(r#"
        [10, 20, 30, 40].indexOf(30)
    "#)
    .expect_number(2.0);
}

/// Verifies index of not found.
#[test]
fn test_index_of_not_found() {
    ShapeTest::new(r#"
        [10, 20, 30].indexOf(99)
    "#)
    .expect_number(-1.0);
}
