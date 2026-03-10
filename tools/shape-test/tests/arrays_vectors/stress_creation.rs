//! Stress tests for array creation and literals.

use shape_test::shape_test::ShapeTest;

/// Verifies array literal empty.
#[test]
fn test_array_literal_empty() {
    ShapeTest::new(
        r#"function test() { let a = []; a.length() }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array literal single int.
#[test]
fn test_array_literal_single_int() {
    ShapeTest::new(
        r#"function test() { let a = [42]; a.first() }
test()"#,
    )
    .expect_number(42.0);
}

/// Verifies array literal multiple ints.
#[test]
fn test_array_literal_multiple_ints() {
    ShapeTest::new(
        r#"function test() { let a = [1, 2, 3]; a.length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array literal floats.
#[test]
fn test_array_literal_floats() {
    ShapeTest::new(
        r#"function test() { [1.5, 2.5, 3.5].first() }
test()"#,
    )
    .expect_number(1.5);
}

/// Verifies array literal strings.
#[test]
fn test_array_literal_strings() {
    ShapeTest::new(
        r#"function test() { ["a", "b", "c"].length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array literal booleans.
#[test]
fn test_array_literal_booleans() {
    ShapeTest::new(
        r#"function test() { [true, false, true].first() }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array literal mixed int float.
#[test]
fn test_array_literal_mixed_int_float() {
    ShapeTest::new(
        r#"function test() { [1, 2.5, 3].length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array literal nested.
#[test]
fn test_array_literal_nested() {
    ShapeTest::new(
        r#"function test() { [[1, 2], [3, 4]].length() }
test()"#,
    )
    .expect_number(2.0);
}

/// Verifies array literal deeply nested.
#[test]
fn test_array_literal_deeply_nested() {
    ShapeTest::new(
        r#"function test() { [[[1]]].length() }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies array literal ten elements.
#[test]
fn test_array_literal_ten_elements() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].length() }
test()"#,
    )
    .expect_number(10.0);
}

/// Verifies array from expression.
#[test]
fn test_array_from_expression() {
    ShapeTest::new(
        r#"function test() { let x = 5; [x, x + 1, x + 2].last() }
test()"#,
    )
    .expect_number(7.0);
}

/// Verifies array index first.
#[test]
fn test_array_index_first() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30][0] }
test()"#,
    )
    .expect_number(10.0);
}

/// Verifies array index middle.
#[test]
fn test_array_index_middle() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30][1] }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array index last.
#[test]
fn test_array_index_last() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30][2] }
test()"#,
    )
    .expect_number(30.0);
}

/// Verifies array index via variable.
#[test]
fn test_array_index_via_variable() {
    ShapeTest::new(
        r#"function test() { let a = [10, 20, 30]; let i = 1; a[i] }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array index string elements.
#[test]
fn test_array_index_string_elements() {
    ShapeTest::new(
        r#"function test() { ["hello", "world"][1] }
test()"#,
    )
    .expect_string("world");
}

/// Verifies array index bool elements.
#[test]
fn test_array_index_bool_elements() {
    ShapeTest::new(
        r#"function test() { [true, false][1] }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies array index computed.
#[test]
fn test_array_index_computed() {
    ShapeTest::new(
        r#"function test() { let a = [10, 20, 30, 40]; a[1 + 1] }
test()"#,
    )
    .expect_number(30.0);
}

/// Verifies array variable then index.
#[test]
fn test_array_variable_then_index() {
    ShapeTest::new(
        r#"function test() { let a = [100, 200, 300]; a[0] }
test()"#,
    )
    .expect_number(100.0);
}

/// Verifies nested array index.
#[test]
fn test_nested_array_index() {
    ShapeTest::new(
        r#"function test() { let a = [[1, 2], [3, 4]]; a[1][0] }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies nested array index deep.
#[test]
fn test_nested_array_index_deep() {
    ShapeTest::new(
        r#"function test() { let a = [[10, 20], [30, 40]]; a[0][1] }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array length method.
#[test]
fn test_array_length_method() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array length empty.
#[test]
fn test_array_length_empty() {
    ShapeTest::new(
        r#"function test() { [].length() }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array length single.
#[test]
fn test_array_length_single() {
    ShapeTest::new(
        r#"function test() { [99].length() }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies array len builtin.
#[test]
fn test_array_len_builtin() {
    ShapeTest::new(
        r#"function test() { len([1, 2, 3, 4]) }
test()"#,
    )
    .expect_number(4.0);
}

/// Verifies array len empty.
#[test]
fn test_array_len_empty() {
    ShapeTest::new(
        r#"function test() { len([]) }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array len method alias.
#[test]
fn test_array_len_method_alias() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].len() }
test()"#,
    )
    .expect_number(5.0);
}

/// Verifies array length nested array.
#[test]
fn test_array_length_nested_array() {
    ShapeTest::new(
        r#"function test() { [[1], [2], [3]].length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array length string array.
#[test]
fn test_array_length_string_array() {
    ShapeTest::new(
        r#"function test() { ["hello", "world"].length() }
test()"#,
    )
    .expect_number(2.0);
}

/// Verifies array first basic.
#[test]
fn test_array_first_basic() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].first() }
test()"#,
    )
    .expect_number(10.0);
}

/// Verifies array first single element.
#[test]
fn test_array_first_single_element() {
    ShapeTest::new(
        r#"function test() { [42].first() }
test()"#,
    )
    .expect_number(42.0);
}

/// Verifies array first empty returns none.
#[test]
fn test_array_first_empty_returns_none() {
    ShapeTest::new(
        r#"function test() { [].first() }
test()"#,
    )
    .expect_none();
}

/// Verifies array first string.
#[test]
fn test_array_first_string() {
    ShapeTest::new(
        r#"function test() { ["alpha", "beta"].first() }
test()"#,
    )
    .expect_string("alpha");
}

/// Verifies array last basic.
#[test]
fn test_array_last_basic() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].last() }
test()"#,
    )
    .expect_number(30.0);
}

/// Verifies array last single element.
#[test]
fn test_array_last_single_element() {
    ShapeTest::new(
        r#"function test() { [42].last() }
test()"#,
    )
    .expect_number(42.0);
}

/// Verifies array last empty returns none.
#[test]
fn test_array_last_empty_returns_none() {
    ShapeTest::new(
        r#"function test() { [].last() }
test()"#,
    )
    .expect_none();
}

/// Verifies array last string.
#[test]
fn test_array_last_string() {
    ShapeTest::new(
        r#"function test() { ["alpha", "beta"].last() }
test()"#,
    )
    .expect_string("beta");
}

/// Verifies array first last same on single.
#[test]
fn test_array_first_last_same_on_single() {
    ShapeTest::new(
        r#"function test() { let a = [99]; a.first() == a.last() }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array reverse basic.
#[test]
fn test_array_reverse_basic() {
    ShapeTest::new(r#"{ let a = [1, 2, 3].reverse(); a[0] }"#).expect_number(3.0);
}

/// Verifies array reverse last element.
#[test]
fn test_array_reverse_last_element() {
    ShapeTest::new(r#"{ let a = [1, 2, 3].reverse(); a[2] }"#).expect_number(1.0);
}

/// Verifies array reverse single element.
#[test]
fn test_array_reverse_single_element() {
    ShapeTest::new(
        r#"function test() { [42].reverse().first() }
test()"#,
    )
    .expect_number(42.0);
}

/// Verifies array reverse empty.
#[test]
fn test_array_reverse_empty() {
    ShapeTest::new(
        r#"function test() { [].reverse().length() }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array reverse preserves length.
#[test]
fn test_array_reverse_preserves_length() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].reverse().length() }
test()"#,
    )
    .expect_number(5.0);
}

/// Verifies array reverse strings.
#[test]
fn test_array_reverse_strings() {
    ShapeTest::new(
        r#"function test() { ["a", "b", "c"].reverse().first() }
test()"#,
    )
    .expect_string("c");
}

/// Verifies array reverse double is identity.
#[test]
fn test_array_reverse_double_is_identity() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].reverse().reverse().first() }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies array slice full.
#[test]
fn test_array_slice_full() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4].slice(0, 4).length() }
test()"#,
    )
    .expect_number(4.0);
}

/// Verifies array slice first two.
#[test]
fn test_array_slice_first_two() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30, 40].slice(0, 2).last() }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array slice middle.
#[test]
fn test_array_slice_middle() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30, 40].slice(1, 3).length() }
test()"#,
    )
    .expect_number(2.0);
}

/// Verifies array slice middle values.
#[test]
fn test_array_slice_middle_values() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30, 40].slice(1, 3).first() }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array slice from start only.
#[test]
fn test_array_slice_from_start_only() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].slice(1).length() }
test()"#,
    )
    .expect_number(2.0);
}

/// Verifies array slice from start only first.
#[test]
fn test_array_slice_from_start_only_first() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].slice(1).first() }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array slice empty result.
#[test]
fn test_array_slice_empty_result() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].slice(2, 2).length() }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array slice beyond length.
#[test]
fn test_array_slice_beyond_length() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].slice(0, 100).length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array slice start beyond length.
#[test]
fn test_array_slice_start_beyond_length() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].slice(10, 20).length() }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array concat two arrays.
#[test]
fn test_array_concat_two_arrays() {
    ShapeTest::new(
        r#"function test() { [1, 2].concat([3, 4]).length() }
test()"#,
    )
    .expect_number(4.0);
}

/// Verifies array concat values.
#[test]
fn test_array_concat_values() {
    ShapeTest::new(
        r#"function test() { [1, 2].concat([3, 4]).last() }
test()"#,
    )
    .expect_number(4.0);
}

/// Verifies array concat first value.
#[test]
fn test_array_concat_first_value() {
    ShapeTest::new(
        r#"function test() { [1, 2].concat([3, 4]).first() }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies array concat empty left.
#[test]
fn test_array_concat_empty_left() {
    ShapeTest::new(
        r#"function test() { [].concat([1, 2, 3]).length() }
test()"#,
    )
    .expect_number(3.0);
}
