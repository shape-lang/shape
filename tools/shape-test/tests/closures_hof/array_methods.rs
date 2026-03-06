//! Array method tests with closures.
//!
//! Covers: map, filter, reduce, find, some, every, sort, flatMap, forEach,
//! method chaining, captured variables in array methods, identity/boolean maps,
//! string reduce, pipeline simulation, and find-none edge case.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// From programs_closures_hof.rs
// =========================================================================

#[test]
fn array_map_double() {
    ShapeTest::new(
        r#"
        let result = [1, 2, 3].map(|x| x * 2)
        result[0] + result[1] + result[2]
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn array_map_to_strings() {
    ShapeTest::new(
        r#"
        let nums = [1, 2, 3]
        let mapped = nums.map(|x| x * 10)
        mapped.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn array_map_with_lambda() {
    ShapeTest::new(
        r#"
        let result = [10, 20, 30].map(|x| x + 1)
        result[1]
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn array_filter_greater_than() {
    ShapeTest::new(
        r#"
        let result = [1, 2, 3, 4, 5].filter(|x| x > 3)
        result.length
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn array_filter_even_numbers() {
    ShapeTest::new(
        r#"
        let evens = [1, 2, 3, 4, 5, 6].filter(|x| x % 2 == 0)
        evens.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn array_filter_preserves_values() {
    ShapeTest::new(
        r#"
        let result = [10, 20, 30, 40].filter(|x| x > 15)
        result[0]
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn array_reduce_sum() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4].reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn array_reduce_product() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4].reduce(|acc, x| acc * x, 1)
    "#,
    )
    .expect_number(24.0);
}

#[test]
fn array_reduce_max() {
    ShapeTest::new(
        r#"
        [3, 7, 2, 9, 1].reduce(|acc, x| if x > acc { x } else { acc }, 0)
    "#,
    )
    .expect_number(9.0);
}

#[test]
fn array_find_first_match() {
    ShapeTest::new(
        r#"
        [10, 20, 30].find(|x| x > 15)
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn array_some_true() {
    ShapeTest::new(
        r#"
        [1, 2, 3].some(|x| x > 2)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn array_some_false() {
    ShapeTest::new(
        r#"
        [1, 2, 3].some(|x| x > 10)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn array_every_true() {
    ShapeTest::new(
        r#"
        [2, 4, 6].every(|x| x > 0)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn array_every_false() {
    ShapeTest::new(
        r#"
        [2, 4, 6].every(|x| x > 3)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn array_sort_with_comparator() {
    ShapeTest::new(
        r#"
        let sorted = [3, 1, 4, 1, 5].sort(|a, b| a - b)
        sorted[0]
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn array_sort_descending() {
    ShapeTest::new(
        r#"
        let sorted = [3, 1, 4, 1, 5].sort(|a, b| b - a)
        sorted[0]
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn array_flatmap_basic() {
    ShapeTest::new(
        r#"
        let result = [[1, 2], [3, 4]].flatMap(|arr| arr)
        result.length
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn array_filter_then_map_chain() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5, 6]
            .filter(|x| x % 2 == 0)
            .map(|x| x * 10)
            .reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(120.0);
}

#[test]
fn array_map_with_captured_variable() {
    ShapeTest::new(
        r#"
        let factor = 10
        let result = [1, 2, 3].map(|x| x * factor)
        result[2]
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn array_foreach_output() {
    ShapeTest::new(
        r#"
        [1, 2, 3].forEach(|x| print(x))
    "#,
    )
    .expect_output("1\n2\n3");
}

// =========================================================================
// From programs_closures_and_hof.rs
// =========================================================================

#[test]
fn test_hof_array_map() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3].map(|x| x * 10)
        arr[2]
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_hof_array_filter() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3, 4, 5, 6].filter(|x| x > 3)
        arr.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_hof_array_reduce() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4].reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_hof_array_map_filter_chain() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5, 6]
            .filter(|x| x % 2 == 0)
            .map(|x| x * 10)
            .reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(120.0);
}

#[test]
fn test_hof_array_find() {
    ShapeTest::new(
        r#"
        [10, 20, 30].find(|x| x > 15)
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_hof_array_some() {
    ShapeTest::new(
        r#"
        [1, 2, 3].some(|x| x > 2)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_hof_array_every() {
    ShapeTest::new(
        r#"
        [2, 4, 6].every(|x| x > 0)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_hof_array_every_false() {
    ShapeTest::new(
        r#"
        [2, 4, 6].every(|x| x > 3)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_hof_array_some_false() {
    ShapeTest::new(
        r#"
        [1, 2, 3].some(|x| x > 10)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_hof_array_flatmap() {
    ShapeTest::new(
        r#"
        [[1, 2], [3, 4]].flatMap(|arr| arr).length
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn test_hof_nested_map() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3].map(|x| x + 1).map(|x| x * 10)
        arr[0]
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_hof_reduce_strings() {
    ShapeTest::new(
        r#"
        ["a", "b", "c"].reduce(|acc, x| acc + x, "")
    "#,
    )
    .expect_string("abc");
}

#[test]
fn test_hof_filter_then_length() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5].filter(|x| x % 2 == 0).length
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn test_hof_reduce_product() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5].reduce(|acc, x| acc * x, 1)
    "#,
    )
    .expect_number(120.0);
}

#[test]
fn test_hof_filter_empty_result() {
    ShapeTest::new(
        r#"
        [1, 2, 3].filter(|x| x > 100).length
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn test_hof_map_identity() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30].map(|x| x)
        arr[1]
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_hof_map_to_bool() {
    ShapeTest::new(
        r#"
        let result = [1, 2, 3, 4, 5].map(|x| x > 3)
        result[3]
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_hof_array_map_first_element() {
    ShapeTest::new(
        r#"
        [5, 10, 15].map(|x| x + 1)[0]
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_hof_array_map_last_element() {
    ShapeTest::new(
        r#"
        [5, 10, 15].map(|x| x * 3).last()
    "#,
    )
    .expect_number(45.0);
}

#[test]
fn test_hof_find_none() {
    // find with no match returns None
    ShapeTest::new(
        r#"
        let result = [1, 2, 3].find(|x| x > 100)
        result == None
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_hof_pipeline_simulation() {
    ShapeTest::new(
        r#"
        let ops = [|x| x + 1, |x| x * 2, |x| x - 3]
        let result = ops.reduce(|val, f| f(val), 5)
        result
    "#,
    )
    .expect_number(9.0);
}
