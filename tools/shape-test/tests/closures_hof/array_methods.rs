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
    // A named comparator is used here: closure *literal* params typed
    // `(T, T)` are not inferred by the current inference engine, so the
    // `|a, b| a - b` form does not type-check — a separate, pre-existing
    // inference limitation independent of `Vec.sort`.
    ShapeTest::new(
        r#"
        fn asc(a: int, b: int) -> int { a - b }
        let sorted = [3, 1, 4, 1, 5].sort(asc)
        sorted[0]
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn array_sort_descending() {
    ShapeTest::new(
        r#"
        fn desc(a: int, b: int) -> int { b - a }
        let sorted = [3, 1, 4, 1, 5].sort(desc)
        sorted[0]
    "#,
    )
    .expect_number(5.0);
}

// =========================================================================
// `Vec.sort` regression coverage (r5c-2-eps-2-stdlib-sort).
//
// The previous `Vec.sort` body was `self.sort(cmp)` (annotated "keep Rust
// delegation"), which method dispatch resolves back to the same Shape
// method — unconditional infinite self-recursion, so any `.sort(...)`
// call overflowed the stack. The body is now a real pure-Shape in-place
// insertion sort. These tests pass a named-function comparator: closure
// *literal* params typed `(T, T)` are not inferred by the current
// inference engine, so the `|a, b| a - b` form above is a separate,
// pre-existing inference limitation tracked independently.
// =========================================================================

#[test]
fn array_sort_named_comparator_ascending() {
    // [3, 1, 4, 1, 5] ascending -> [1, 1, 3, 4, 5]; first three digits.
    ShapeTest::new(
        r#"
        fn asc(a: int, b: int) -> int { a - b }
        let sorted = [3, 1, 4, 1, 5].sort(asc)
        sorted[0] * 100 + sorted[1] * 10 + sorted[2]
    "#,
    )
    .expect_number(113.0);
}

#[test]
fn array_sort_named_comparator_full_order() {
    ShapeTest::new(
        r#"
        fn asc(a: int, b: int) -> int { a - b }
        let sorted = [9, 8, 7, 6, 5, 4, 3, 2, 1, 0].sort(asc)
        let mut ok = 0
        let mut i = 0
        while i < 10 {
            if sorted[i] == i { ok = ok + 1 }
            i = i + 1
        }
        ok
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn array_sort_named_comparator_descending() {
    ShapeTest::new(
        r#"
        fn desc(a: int, b: int) -> int { b - a }
        let sorted = [3, 1, 4, 1, 5].sort(desc)
        sorted[0] * 100 + sorted[1] * 10 + sorted[4]
    "#,
    )
    .expect_number(541.0);
}

#[test]
fn array_sort_single_element() {
    ShapeTest::new(
        r#"
        fn asc(a: int, b: int) -> int { a - b }
        let sorted = [42].sort(asc)
        sorted[0]
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn array_sort_two_elements_swapped() {
    ShapeTest::new(
        r#"
        fn asc(a: int, b: int) -> int { a - b }
        let sorted = [2, 1].sort(asc)
        sorted[0] * 10 + sorted[1]
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn array_sort_with_duplicates_preserves_count() {
    ShapeTest::new(
        r#"
        fn asc(a: int, b: int) -> int { a - b }
        let sorted = [5, 5, 3, 5, 1].sort(asc)
        sorted[0] * 10000 + sorted[1] * 1000 + sorted[2] * 100
            + sorted[3] * 10 + sorted[4]
    "#,
    )
    .expect_number(13555.0);
}

#[test]
fn array_sort_does_not_mutate_receiver() {
    // `sort` returns a new array; the source is untouched.
    ShapeTest::new(
        r#"
        fn asc(a: int, b: int) -> int { a - b }
        let source = [3, 1, 2]
        let sorted = source.sort(asc)
        source[0] * 100 + source[1] * 10 + source[2]
    "#,
    )
    .expect_number(312.0);
}

#[test]
fn array_sort_stable_for_equal_keys() {
    // Insertion sort only shifts a neighbour when it is *strictly*
    // greater than the key, so equal elements never cross — a comparator
    // that treats everything as equal must leave the array unchanged.
    ShapeTest::new(
        r#"
        fn eq(a: int, b: int) -> int { 0 }
        let sorted = [4, 1, 3, 2].sort(eq)
        sorted[0] * 1000 + sorted[1] * 100 + sorted[2] * 10 + sorted[3]
    "#,
    )
    .expect_number(4132.0);
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
    // forEach runs the callback for side effects. The output capture mechanism
    // does not collect print calls made inside forEach callbacks (they go to
    // stdout directly), so we verify it runs without error.
    ShapeTest::new(
        r#"
        [1, 2, 3].forEach(|x| print(x))
    "#,
    )
    .expect_run_ok();
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
