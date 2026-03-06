//! Stress tests for array reduce, fold, and aggregation.

use shape_test::shape_test::ShapeTest;


/// Verifies distinct alias.
#[test]
fn test_distinct_alias() {
    ShapeTest::new(r#"(
        [1, 1, 2, 2, 3].distinct()
    )[0]"#)
    .expect_number(1.0);
    ShapeTest::new(r#"(
        [1, 1, 2, 2, 3].distinct()
    )[2]"#)
    .expect_number(3.0);
}

/// Verifies unique two elements same.
#[test]
fn test_unique_two_elements_same() {
    ShapeTest::new(r#"(
        [7, 7].unique()
    )[0]"#)
    .expect_number(7.0);
}

/// Verifies flatmap expand.
#[test]
fn test_flatmap_expand() {
    ShapeTest::new(r#"(
        [1, 2, 3].flatMap(|x| [x, x * 10])
    )[0]"#)
    .expect_number(1.0);
    ShapeTest::new(r#"(
        [1, 2, 3].flatMap(|x| [x, x * 10])
    )[5]"#)
    .expect_number(30.0);
}

/// Verifies flatmap identity nested.
#[test]
fn test_flatmap_identity_nested() {
    ShapeTest::new(r#"(
        [[1, 2], [3, 4], [5]].flatMap(|x| x)
    )[0]"#)
    .expect_number(1.0);
    ShapeTest::new(r#"(
        [[1, 2], [3, 4], [5]].flatMap(|x| x)
    )[4]"#)
    .expect_number(5.0);
}

/// Verifies flatmap empty results.
#[test]
fn test_flatmap_empty_results() {
    ShapeTest::new(r#"
        let empty = []
        [1, 2, 3].flatMap(|x| empty).length
    "#)
    .expect_number(0.0);
}

/// Verifies flatmap single element arrays.
#[test]
fn test_flatmap_single_element_arrays() {
    ShapeTest::new(r#"(
        [10, 20, 30].flatMap(|x| [x])
    )[0]"#)
    .expect_number(10.0);
    ShapeTest::new(r#"(
        [10, 20, 30].flatMap(|x| [x])
    )[2]"#)
    .expect_number(30.0);
}

/// Verifies flatmap triple.
#[test]
fn test_flatmap_triple() {
    ShapeTest::new(r#"(
        [1, 2].flatMap(|x| [x, x, x])
    )[0]"#)
    .expect_number(1.0);
    ShapeTest::new(r#"(
        [1, 2].flatMap(|x| [x, x, x])
    )[5]"#)
    .expect_number(2.0);
}

/// Verifies flatmap on empty.
#[test]
fn test_flatmap_on_empty() {
    ShapeTest::new(r#"
        let empty = []
        empty.flatMap(|x| [x, x]).length
    "#)
    .expect_number(0.0);
}

/// Verifies find first match.
#[test]
fn test_find_first_match() {
    ShapeTest::new(r#"
        [1, 2, 3, 4].find(|x| x > 2)
    "#)
    .expect_number(3.0);
}

/// Verifies find no match.
#[test]
fn test_find_no_match() {
    ShapeTest::new(r#"
        [1, 2, 3].find(|x| x > 10)
    "#)
    .expect_none();
}

/// Verifies find first element.
#[test]
fn test_find_first_element() {
    ShapeTest::new(r#"
        [10, 20, 30].find(|x| x > 0)
    "#)
    .expect_number(10.0);
}

/// Verifies find on empty.
#[test]
fn test_find_on_empty() {
    ShapeTest::new(r#"
        let empty = []
        empty.find(|x| x > 0)
    "#)
    .expect_none();
}

/// Verifies find index found.
#[test]
fn test_find_index_found() {
    ShapeTest::new(r#"
        [10, 20, 30, 40].findIndex(|x| x > 25)
    "#)
    .expect_number(2.0);
}

/// Verifies find index not found.
#[test]
fn test_find_index_not_found() {
    ShapeTest::new(r#"
        [1, 2, 3].findIndex(|x| x > 10)
    "#)
    .expect_number(-1.0);
}

/// Verifies find index first.
#[test]
fn test_find_index_first() {
    ShapeTest::new(r#"
        [5, 10, 15].findIndex(|x| x == 5)
    "#)
    .expect_number(0.0);
}

/// Verifies find index empty.
#[test]
fn test_find_index_empty() {
    ShapeTest::new(r#"
        let empty = []
        empty.findIndex(|x| x > 0)
    "#)
    .expect_number(-1.0);
}

/// Verifies some true.
#[test]
fn test_some_true() {
    ShapeTest::new(r#"
        [1, 2, 3].some(|x| x > 2)
    "#)
    .expect_bool(true);
}

/// Verifies some false.
#[test]
fn test_some_false() {
    ShapeTest::new(r#"
        [1, 2, 3].some(|x| x > 10)
    "#)
    .expect_bool(false);
}

/// Verifies some empty.
#[test]
fn test_some_empty() {
    ShapeTest::new(r#"
        let empty = []
        empty.some(|x| x > 0)
    "#)
    .expect_bool(false);
}

/// Verifies some all match.
#[test]
fn test_some_all_match() {
    ShapeTest::new(r#"
        [1, 2, 3].some(|x| x > 0)
    "#)
    .expect_bool(true);
}

/// Verifies every true.
#[test]
fn test_every_true() {
    ShapeTest::new(r#"
        [1, 2, 3].every(|x| x > 0)
    "#)
    .expect_bool(true);
}

/// Verifies every false.
#[test]
fn test_every_false() {
    ShapeTest::new(r#"
        [1, 2, 3].every(|x| x > 1)
    "#)
    .expect_bool(false);
}

/// Verifies every empty.
#[test]
fn test_every_empty() {
    ShapeTest::new(r#"
        let empty = []
        empty.every(|x| x > 0)
    "#)
    .expect_bool(true);
}

/// Verifies every single true.
#[test]
fn test_every_single_true() {
    ShapeTest::new(r#"
        [5].every(|x| x > 0)
    "#)
    .expect_bool(true);
}

/// Verifies any alias true.
#[test]
fn test_any_alias_true() {
    ShapeTest::new(r#"
        [1, 2, 3].any(|x| x == 2)
    "#)
    .expect_bool(true);
}

/// Verifies any alias false.
#[test]
fn test_any_alias_false() {
    ShapeTest::new(r#"
        [1, 2, 3].any(|x| x > 5)
    "#)
    .expect_bool(false);
}

/// Verifies all alias true.
#[test]
fn test_all_alias_true() {
    ShapeTest::new(r#"
        [2, 4, 6].all(|x| x % 2 == 0)
    "#)
    .expect_bool(true);
}

/// Verifies all alias false.
#[test]
fn test_all_alias_false() {
    ShapeTest::new(r#"
        [2, 3, 6].all(|x| x % 2 == 0)
    "#)
    .expect_bool(false);
}

/// Verifies count returns length.
#[test]
fn test_count_returns_length() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5].count()
    "#)
    .expect_number(5.0);
}

/// Verifies count empty.
#[test]
fn test_count_empty() {
    ShapeTest::new(r#"
        let empty = []
        empty.count()
    "#)
    .expect_number(0.0);
}

/// Verifies sum basic.
#[test]
fn test_sum_basic() {
    ShapeTest::new(r#"
        [1, 2, 3].sum()
    "#)
    .expect_number(6.0);
}

/// Verifies sum single.
#[test]
fn test_sum_single() {
    ShapeTest::new(r#"
        [42].sum()
    "#)
    .expect_number(42.0);
}

/// Verifies sum negative.
#[test]
fn test_sum_negative() {
    ShapeTest::new(r#"
        [-1, -2, -3].sum()
    "#)
    .expect_number(-6.0);
}

/// Verifies sum floats.
#[test]
fn test_sum_floats() {
    ShapeTest::new(r#"
        [1.5, 2.5, 3.0].sum()
    "#)
    .expect_number(7.0);
}

/// Verifies avg basic.
#[test]
fn test_avg_basic() {
    ShapeTest::new(r#"
        [1, 2, 3].avg()
    "#)
    .expect_number(2.0);
}

/// Verifies avg single.
#[test]
fn test_avg_single() {
    ShapeTest::new(r#"
        [10].avg()
    "#)
    .expect_number(10.0);
}

/// Verifies avg empty.
#[test]
fn test_avg_empty() {
    ShapeTest::new(r#"
        let empty = []
        empty.avg()
    "#)
    .expect_number(0.0);
}

/// Verifies avg same values.
#[test]
fn test_avg_same_values() {
    ShapeTest::new(r#"
        [5, 5, 5, 5].avg()
    "#)
    .expect_number(5.0);
}

/// Verifies min basic.
#[test]
fn test_min_basic() {
    ShapeTest::new(r#"
        [3, 1, 2].min()
    "#)
    .expect_number(1.0);
}

/// Verifies min negative.
#[test]
fn test_min_negative() {
    ShapeTest::new(r#"
        [5, -3, 0, 2].min()
    "#)
    .expect_number(-3.0);
}

/// Verifies min single.
#[test]
fn test_min_single() {
    ShapeTest::new(r#"
        [7].min()
    "#)
    .expect_number(7.0);
}

/// Verifies max basic.
#[test]
fn test_max_basic() {
    ShapeTest::new(r#"
        [3, 1, 2].max()
    "#)
    .expect_number(3.0);
}

/// Verifies max negative.
#[test]
fn test_max_negative() {
    ShapeTest::new(r#"
        [-5, -3, -1].max()
    "#)
    .expect_number(-1.0);
}
