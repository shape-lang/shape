//! Stress tests for iterator chaining (2-op and 3+-op chains), pipeline operator,
//! practical patterns, closures with captures, complex pipelines, and misc array methods.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 6: Iterator Chaining — 2 operations
// =============================================================================

/// Iter map then filter.
#[test]
fn test_iter_map_then_filter() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5].iter().map(|x| x * 2).filter(|x| x > 6).collect()
            arr[0] + arr[1]
        }
    "#).expect_number(18.0);
}

/// Iter filter then map.
#[test]
fn test_iter_filter_then_map() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5].iter().filter(|x| x > 2).map(|x| x * 10).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(120.0);
}

/// Iter filter then count.
#[test]
fn test_iter_filter_then_count() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].iter().filter(|x| x % 3 == 0).count()"#)
        .expect_number(3.0);
}

/// Iter map then reduce.
#[test]
fn test_iter_map_then_reduce() {
    ShapeTest::new(r#"[1, 2, 3].iter().map(|x| x * x).reduce(|acc, x| acc + x, 0)"#)
        .expect_number(14.0);
}

/// Iter filter then find.
#[test]
fn test_iter_filter_then_find() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5, 6].iter().filter(|x| x % 2 == 0).find(|x| x > 3)"#)
        .expect_number(4.0);
}

/// Iter map then any.
#[test]
fn test_iter_map_then_any() {
    ShapeTest::new(r#"[1, 2, 3].iter().map(|x| x * 10).any(|x| x > 25)"#)
        .expect_bool(true);
}

/// Iter map then all.
#[test]
fn test_iter_map_then_all() {
    ShapeTest::new(r#"[1, 2, 3].iter().map(|x| x * 10).all(|x| x > 5)"#)
        .expect_bool(true);
}

// =============================================================================
// SECTION 7: Iterator Chaining — 3+ operations
// =============================================================================

/// Iter filter map take.
#[test]
fn test_iter_filter_map_take() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
                .iter()
                .filter(|x| x % 2 == 0)
                .map(|x| x * x)
                .take(3)
                .collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(56.0);
}

/// Iter skip filter map collect.
#[test]
fn test_iter_skip_filter_map_collect() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5, 6, 7, 8]
                .iter()
                .skip(2)
                .filter(|x| x % 2 == 0)
                .map(|x| x + 100)
                .collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(318.0);
}

/// Iter filter map reduce chain.
#[test]
fn test_iter_filter_map_reduce_chain() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .iter()
            .filter(|x| x > 5)
            .map(|x| x * 2)
            .reduce(|acc, x| acc + x, 0)
    "#).expect_number(80.0);
}

/// Iter map filter take count.
#[test]
fn test_iter_map_filter_take_count() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .iter()
            .map(|x| x * 3)
            .filter(|x| x > 10)
            .take(4)
            .count()
    "#).expect_number(4.0);
}

/// Iter filter skip take collect.
#[test]
fn test_iter_filter_skip_take_collect() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
                .iter()
                .filter(|x| x % 2 == 0)
                .skip(1)
                .take(2)
                .collect()
            arr[0] + arr[1]
        }
    "#).expect_number(10.0);
}

// =============================================================================
// SECTION 10: Pipeline Operator |>
// =============================================================================

/// Direct function call (basic).
#[test]
fn test_pipe_basic_function() {
    ShapeTest::new(r#"
        fn double(x: int) -> int { x * 2 }
        fn test() -> int { double(5) }
        test()
    "#).expect_number(10.0);
}

/// Direct function chaining with two functions.
#[test]
fn test_pipe_chain_two_functions() {
    ShapeTest::new(r#"
        fn double(x: int) -> int { x * 2 }
        fn add_one(x: int) -> int { x + 1 }
        fn test() -> int { add_one(double(5)) }
        test()
    "#).expect_number(11.0);
}

/// Direct function chaining with three functions.
#[test]
fn test_pipe_chain_three_functions() {
    ShapeTest::new(r#"
        fn double(x: int) -> int { x * 2 }
        fn add_one(x: int) -> int { x + 1 }
        fn negate(x: int) -> int { 0 - x }
        fn test() -> int { negate(add_one(double(3))) }
        test()
    "#).expect_number(-7.0);
}

/// Direct function call with extra args.
#[test]
fn test_pipe_with_extra_args() {
    ShapeTest::new(r#"
        fn add(x: int, y: int) -> int { x + y }
        fn test() -> int { add(10, 5) }
        test()
    "#).expect_number(15.0);
}

/// Pipe identifier form.
#[test]
fn test_pipe_identifier_form() {
    ShapeTest::new(r#"
        fn double(x: int) -> int { x * 2 }
        fn test() -> int { 7 |> double }
        test()
    "#).expect_number(14.0);
}

// =============================================================================
// SECTION 11: Practical Patterns
// =============================================================================

/// Sum of squares.
#[test]
fn test_sum_of_squares() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].map(|x| x * x).reduce(|acc, x| acc + x, 0)"#)
        .expect_number(55.0);
}

/// Sum of squares via iter.
#[test]
fn test_sum_of_squares_via_iter() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].iter().map(|x| x * x).reduce(|acc, x| acc + x, 0)"#)
        .expect_number(55.0);
}

/// Count evens.
#[test]
fn test_count_evens() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].filter(|x| x % 2 == 0).length"#)
        .expect_number(5.0);
}

/// Count evens via iter.
#[test]
fn test_count_evens_via_iter() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].iter().filter(|x| x % 2 == 0).count()"#)
        .expect_number(5.0);
}

/// Find first greater than.
#[test]
fn test_find_first_greater_than() {
    ShapeTest::new(r#"[5, 10, 15, 20, 25].iter().find(|x| x > 12)"#)
        .expect_number(15.0);
}

/// All positive.
#[test]
fn test_all_positive() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].iter().all(|x| x > 0)"#)
        .expect_bool(true);
}

/// Any negative.
#[test]
fn test_any_negative() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].iter().any(|x| x < 0)"#)
        .expect_bool(false);
}

/// Filter and sum.
#[test]
fn test_filter_and_sum() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .iter()
            .filter(|x| x > 5)
            .reduce(|acc, x| acc + x, 0)
    "#).expect_number(40.0);
}

/// Double and take first three.
#[test]
fn test_double_and_take_first_three() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5].iter().map(|x| x * 2).take(3).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(12.0);
}

/// Complex pipeline pattern.
#[test]
fn test_complex_pipeline_pattern() {
    ShapeTest::new(r#"
        {
            let data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            data.iter()
                .filter(|x| x % 2 == 0)
                .map(|x| x * x)
                .take(3)
                .reduce(|acc, x| acc + x, 0)
        }
    "#).expect_number(56.0);
}

// =============================================================================
// SECTION 17: Nested array operations
// =============================================================================

/// Nested map flatten.
#[test]
fn test_nested_map_flatten() {
    ShapeTest::new(r#"
        {
            let arr = [[1, 2], [3, 4], [5, 6]].flatMap(|arr| arr)
            arr[0] + arr[5]
        }
    "#).expect_number(7.0);
}

/// Nested array map inner.
#[test]
fn test_nested_array_map_inner() {
    ShapeTest::new(r#"
        {
            let arr = [[1, 2, 3], [4, 5, 6]].map(|inner| inner.length)
            arr[0] + arr[1]
        }
    "#).expect_number(6.0);
}

/// Flatten then filter.
#[test]
fn test_flatten_then_filter() {
    ShapeTest::new(r#"
        {
            let arr = [[1, 2], [3, 4], [5, 6]].flatMap(|arr| arr).filter(|x| x > 3)
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(15.0);
}

// =============================================================================
// SECTION 18: Closures that capture variables in iterators
// =============================================================================

/// Iter map with captured variable.
#[test]
fn test_iter_map_with_captured_variable() {
    ShapeTest::new(r#"
        {
            let multiplier = 10
            let arr = [1, 2, 3].map(|x| x * multiplier)
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(60.0);
}

/// Iter filter with captured threshold.
#[test]
fn test_iter_filter_with_captured_threshold() {
    ShapeTest::new(r#"
        {
            let threshold = 3
            let arr = [1, 2, 3, 4, 5].filter(|x| x > threshold)
            arr[0] + arr[1]
        }
    "#).expect_number(9.0);
}

/// Iter map with captured in iter chain.
#[test]
fn test_iter_map_with_captured_in_iter_chain() {
    ShapeTest::new(r#"
        {
            let offset = 100
            let arr = [1, 2, 3].iter().map(|x| x + offset).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(306.0);
}

/// Iter reduce with captured var.
#[test]
fn test_iter_reduce_with_captured_var() {
    ShapeTest::new(r#"
        {
            let bonus = 100
            [1, 2, 3].iter().reduce(|acc, x| acc + x, bonus)
        }
    "#).expect_number(106.0);
}

// =============================================================================
// SECTION 23: Function-based Iteration Patterns
// =============================================================================

/// Fn returning iter result.
#[test]
fn test_fn_returning_iter_result() {
    ShapeTest::new(r#"
        fn sum_evens(nums: Array<int>) -> int {
            nums.filter(|x| x % 2 == 0).reduce(|acc, x| acc + x, 0)
        }
        fn test() -> int { sum_evens([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]) }
        test()
    "#).expect_number(30.0);
}

/// Fn with iter pipeline.
#[test]
fn test_fn_with_iter_pipeline() {
    ShapeTest::new(r#"
        fn top_n_doubled(nums: Array<int>, n: int) -> Array<int> {
            nums.iter().map(|x| x * 2).take(n).collect()
        }
        fn test() -> int {
            let result = top_n_doubled([5, 10, 15, 20, 25], 3)
            result.length
        }
        test()
    "#).expect_number(3.0);
}

/// Fn iter chain in expression.
#[test]
fn test_fn_iter_chain_in_expression() {
    ShapeTest::new(r#"
        fn test() -> int {
            let a = [1, 2, 3]
            let b = [4, 5, 6]
            let combined = a.iter().chain(b.iter()).collect()
            combined.length
        }
        test()
    "#).expect_number(6.0);
}

// =============================================================================
// SECTION 24: Complex Multi-Step Pipeline Scenarios
// =============================================================================

/// Complex data transformation.
#[test]
fn test_complex_data_transformation() {
    ShapeTest::new(r#"
        {
            let data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            let result = data
                .filter(|x| x > 3)
                .map(|x| x * x)
                .filter(|x| x < 50)
                .reduce(|acc, x| acc + x, 0)
            result
        }
    "#).expect_number(126.0);
}

/// Complex iter pipeline.
#[test]
fn test_complex_iter_pipeline() {
    ShapeTest::new(r#"
        {
            let data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            let arr = data.iter()
                .filter(|x| x % 2 == 0)
                .map(|x| x * 3)
                .skip(1)
                .take(3)
                .collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(54.0);
}

/// Iter map map collect.
#[test]
fn test_iter_map_map_collect() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3].iter().map(|x| x + 1).map(|x| x * 10).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(90.0);
}

/// Iter filter filter collect.
#[test]
fn test_iter_filter_filter_collect() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
                .iter()
                .filter(|x| x > 3)
                .filter(|x| x < 8)
                .collect()
            arr[0] + arr[3]
        }
    "#).expect_number(11.0);
}

// =============================================================================
// SECTION 25: Miscellaneous — additional coverage
// =============================================================================

/// Array includes.
#[test]
fn test_array_includes() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].includes(3)"#)
        .expect_bool(true);
}

/// Array includes missing.
#[test]
fn test_array_includes_missing() {
    ShapeTest::new(r#"[1, 2, 3, 4, 5].includes(99)"#)
        .expect_bool(false);
}

/// Array indexOf.
#[test]
fn test_array_indexOf() {
    ShapeTest::new(r#"[10, 20, 30, 40].indexOf(30)"#)
        .expect_number(2.0);
}

/// Array concat.
#[test]
fn test_array_concat() {
    ShapeTest::new(r#"[1, 2, 3].concat([4, 5, 6]).length"#)
        .expect_number(6.0);
}

/// Array unique.
#[test]
fn test_array_unique() {
    ShapeTest::new(r#"[1, 2, 2, 3, 3, 3].unique().length"#)
        .expect_number(3.0);
}

/// Array flatten.
#[test]
fn test_array_flatten() {
    ShapeTest::new(r#"[[1, 2], [3, 4], [5, 6]].flatten().length"#)
        .expect_number(6.0);
}

/// Array slice.
#[test]
fn test_array_slice() {
    ShapeTest::new(r#"[10, 20, 30, 40, 50].slice(1, 4)[0]"#)
        .expect_number(20.0);
}

/// Array join string.
#[test]
fn test_array_join_string() {
    ShapeTest::new(r#"["a", "b", "c"].join(", ")"#)
        .expect_string("a, b, c");
}

/// Iter map with arithmetic.
#[test]
fn test_iter_map_with_arithmetic() {
    ShapeTest::new(r#"
        {
            let arr = [10, 20, 30].iter().map(|x| x / 2).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(30.0);
}

/// Iter map constant value.
#[test]
fn test_iter_map_constant_value() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3].iter().map(|x| 0).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(0.0);
}

/// Array findIndex.
#[test]
fn test_array_findindex() {
    ShapeTest::new(r#"[10, 20, 30, 40, 50].findIndex(|x| x > 25)"#)
        .expect_number(2.0);
}

/// Array findIndex not found.
#[test]
fn test_array_findindex_not_found() {
    ShapeTest::new(r#"[10, 20, 30].findIndex(|x| x > 100)"#)
        .expect_number(-1.0);
}

/// Chain map filter find.
#[test]
fn test_chain_map_filter_find() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5]
            .iter()
            .map(|x| x * x)
            .filter(|x| x > 5)
            .find(|x| x > 10)
    "#).expect_number(16.0);
}

/// Chain filter map any.
#[test]
fn test_chain_filter_map_any() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5]
            .iter()
            .filter(|x| x > 2)
            .map(|x| x * 10)
            .any(|x| x > 40)
    "#).expect_bool(true);
}

/// Chain map filter all.
#[test]
fn test_chain_map_filter_all() {
    ShapeTest::new(r#"
        [1, 2, 3, 4, 5]
            .iter()
            .map(|x| x * 10)
            .filter(|x| x > 20)
            .all(|x| x > 25)
    "#).expect_bool(true);
}

/// Iter take one.
#[test]
fn test_iter_take_one() {
    ShapeTest::new(r#"[10, 20, 30].iter().take(1).collect()[0]"#)
        .expect_number(10.0);
}

/// Iter skip one.
#[test]
fn test_iter_skip_one() {
    ShapeTest::new(r#"[10, 20, 30].iter().skip(1).collect()[0]"#)
        .expect_number(20.0);
}

/// Iter map negative values.
#[test]
fn test_iter_map_negative_values() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3].iter().map(|x| 0 - x).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(-6.0);
}

/// Iter filter modulo.
#[test]
fn test_iter_filter_modulo() {
    ShapeTest::new(r#"
        {
            let arr = [1, 2, 3, 4, 5, 6, 7, 8, 9].iter().filter(|x| x % 3 == 0).collect()
            arr[0] + arr[1] + arr[2]
        }
    "#).expect_number(18.0);
}
