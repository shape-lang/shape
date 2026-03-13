//! Stress tests for array mutation and control flow.

use shape_test::shape_test::ShapeTest;

/// Verifies array chain take last.
#[test]
fn test_array_chain_take_last() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].take(3).last() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array chain drop first.
#[test]
fn test_array_chain_drop_first() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].drop(2).first() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array chain concat length.
#[test]
fn test_array_chain_concat_length() {
    ShapeTest::new(
        r#"function test() { [1, 2].concat([3, 4]).concat([5]).length() }
test()"#,
    )
    .expect_number(5.0);
}

/// Verifies array chain slice reverse.
#[test]
fn test_array_chain_slice_reverse() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].slice(1, 4).reverse().first() }
test()"#,
    )
    .expect_number(4.0);
}

/// Verifies array chain take reverse first.
#[test]
fn test_array_chain_take_reverse_first() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30, 40].take(3).reverse().first() }
test()"#,
    )
    .expect_number(30.0);
}

/// Verifies array length equality.
#[test]
fn test_array_length_equality() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].length() == 3 }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array element equality.
#[test]
fn test_array_element_equality() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30][1] == 20 }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array large build with loop.
#[test]
fn test_array_large_build_with_loop() {
    ShapeTest::new(
        r#"function test() {
            let mut a = []
            let mut i = 0
            while i < 100 {
                a = a.push(i)
                i = i + 1
            }
            a.length()
        }
test()"#,
    )
    .expect_number(100.0);
}

/// Verifies array large last element.
#[test]
fn test_array_large_last_element() {
    ShapeTest::new(
        r#"function test() {
            let mut a = []
            let mut i = 0
            while i < 50 {
                a = a.push(i)
                i = i + 1
            }
            a.last()
        }
test()"#,
    )
    .expect_number(49.0);
}

/// Verifies array large first element.
#[test]
fn test_array_large_first_element() {
    ShapeTest::new(
        r#"function test() {
            let mut a = []
            let mut i = 0
            while i < 50 {
                a = a.push(i)
                i = i + 1
            }
            a.first()
        }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array large index access.
#[test]
fn test_array_large_index_access() {
    ShapeTest::new(
        r#"function test() {
            let mut a = []
            let mut i = 0
            while i < 100 {
                a = a.push(i * 2)
                i = i + 1
            }
            a[50]
        }
test()"#,
    )
    .expect_number(100.0);
}

/// Verifies array passed to function.
#[test]
fn test_array_passed_to_function() {
    ShapeTest::new(
        r#"function sum_arr(arr) {
            let mut s = 0
            for x in arr {
                s = s + x
            }
            s
        }
        function test() { sum_arr([1, 2, 3, 4]) }
test()"#,
    )
    .expect_number(10.0);
}

/// Verifies array returned from function.
#[test]
fn test_array_returned_from_function() {
    ShapeTest::new(
        r#"function make_arr() { [10, 20, 30] }
        function test() { make_arr().length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array returned element access.
#[test]
fn test_array_returned_element_access() {
    ShapeTest::new(
        r#"function make_arr() { [10, 20, 30] }
        function test() { make_arr()[1] }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array single element operations.
#[test]
fn test_array_single_element_operations() {
    ShapeTest::new(
        r#"function test() { [42].take(1).drop(0).reverse().first() }
test()"#,
    )
    .expect_number(42.0);
}

/// Verifies array empty chaining.
#[test]
fn test_array_empty_chaining() {
    ShapeTest::new(
        r#"function test() { [].concat([]).take(0).drop(0).length() }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array take then drop.
#[test]
fn test_array_take_then_drop() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].take(4).drop(2).length() }
test()"#,
    )
    .expect_number(2.0);
}

/// Verifies array take then drop values.
#[test]
fn test_array_take_then_drop_values() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].take(4).drop(2).first() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array index zero.
#[test]
fn test_array_index_zero() {
    ShapeTest::new(
        r#"function test() { [99][0] }
test()"#,
    )
    .expect_number(99.0);
}

/// Verifies array concat single element arrays.
#[test]
fn test_array_concat_single_element_arrays() {
    ShapeTest::new(
        r#"function test() { [1].concat([2]).concat([3]).length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array concat single element arrays values.
#[test]
fn test_array_concat_single_element_arrays_values() {
    ShapeTest::new(
        r#"function test() { [1].concat([2]).concat([3]).last() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array flatten single nested.
#[test]
fn test_array_flatten_single_nested() {
    ShapeTest::new(
        r#"function test() { [[1, 2, 3]].flatten().length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array flatten preserves order.
#[test]
fn test_array_flatten_preserves_order() {
    ShapeTest::new(
        r#"function test() { [[3, 4], [1, 2]].flatten().first() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array slice single element.
#[test]
fn test_array_slice_single_element() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].slice(1, 2).first() }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array join with space.
#[test]
fn test_array_join_with_space() {
    ShapeTest::new(
        r#"function test() { ["hello", "world"].join(" ") }
test()"#,
    )
    .expect_string("hello world");
}

/// Verifies array index assignment.
#[test]
fn test_array_index_assignment() {
    ShapeTest::new(
        r#"function test() {
            let mut a = [1, 2, 3]
            a[1] = 99
            a[1]
        }
test()"#,
    )
    .expect_number(99.0);
}

/// Verifies array index assignment first.
#[test]
fn test_array_index_assignment_first() {
    ShapeTest::new(
        r#"function test() {
            let mut a = [10, 20, 30]
            a[0] = 99
            a[0]
        }
test()"#,
    )
    .expect_number(99.0);
}

/// Verifies array index assignment last.
#[test]
fn test_array_index_assignment_last() {
    ShapeTest::new(
        r#"function test() {
            let mut a = [10, 20, 30]
            a[2] = 99
            a[2]
        }
test()"#,
    )
    .expect_number(99.0);
}

/// Verifies array index assignment preserves others.
#[test]
fn test_array_index_assignment_preserves_others() {
    ShapeTest::new(
        r#"function test() {
            let mut a = [10, 20, 30]
            a[1] = 99
            a[0]
        }
test()"#,
    )
    .expect_number(10.0);
}

/// Verifies array for in collect sum.
#[test]
fn test_array_for_in_collect_sum() {
    ShapeTest::new(
        r#"function test() {
            let mut total = 0
            for v in [100, 200, 300] {
                total = total + v
            }
            total
        }
test()"#,
    )
    .expect_number(600.0);
}

/// Verifies array for in count.
#[test]
fn test_array_for_in_count() {
    ShapeTest::new(
        r#"function test() {
            let mut count = 0
            for _ in [1, 2, 3, 4, 5] {
                count = count + 1
            }
            count
        }
test()"#,
    )
    .expect_number(5.0);
}

/// Verifies array for in empty.
#[test]
fn test_array_for_in_empty() {
    ShapeTest::new(
        r#"function test() {
            let mut count = 0
            for _ in [] {
                count = count + 1
            }
            count
        }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array for in strings.
#[test]
fn test_array_for_in_strings() {
    ShapeTest::new(
        r#"function test() {
            let mut result = ""
            for s in ["a", "b", "c"] {
                result = result + s
            }
            result
        }
test()"#,
    )
    .expect_string("abc");
}

/// Verifies array of zero.
#[test]
fn test_array_of_zero() {
    ShapeTest::new(
        r#"function test() { [0, 0, 0].length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array of negative ints.
#[test]
fn test_array_of_negative_ints() {
    ShapeTest::new(
        r#"function test() { [-1, -2, -3].first() }
test()"#,
    )
    .expect_number(-1.0);
}

/// Verifies array of negative ints last.
#[test]
fn test_array_of_negative_ints_last() {
    ShapeTest::new(
        r#"function test() { [-10, -20, -30].last() }
test()"#,
    )
    .expect_number(-30.0);
}

/// Verifies array of large ints.
#[test]
fn test_array_of_large_ints() {
    ShapeTest::new(
        r#"function test() { [1000000, 2000000].first() }
test()"#,
    )
    .expect_number(1000000.0);
}

/// Verifies array contains none.
#[test]
fn test_array_contains_none() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].includes(None) }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies nested array lengths.
#[test]
fn test_nested_array_lengths() {
    ShapeTest::new(
        r#"function test() { [[1, 2, 3], [4, 5]].first().length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies nested array inner first.
#[test]
fn test_nested_array_inner_first() {
    ShapeTest::new(
        r#"function test() { [[10, 20], [30, 40]].first().first() }
test()"#,
    )
    .expect_number(10.0);
}

/// Verifies nested array inner last.
#[test]
fn test_nested_array_inner_last() {
    ShapeTest::new(
        r#"function test() { [[10, 20], [30, 40]].last().last() }
test()"#,
    )
    .expect_number(40.0);
}

/// Verifies nested array flatten and index.
#[test]
fn test_nested_array_flatten_and_index() {
    ShapeTest::new(
        r#"function test() { [[1, 2], [3, 4]].flatten()[2] }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array let binding.
#[test]
fn test_array_let_binding() {
    ShapeTest::new(
        r#"function test() { let arr = [5, 10, 15]; arr[1] }
test()"#,
    )
    .expect_number(10.0);
}

/// Verifies array returned as value.
#[test]
fn test_array_returned_as_value() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3] }
test().length"#,
    )
    .expect_number(3.0);
}

/// Verifies array length after concat.
#[test]
fn test_array_length_after_concat() {
    ShapeTest::new(
        r#"function test() { let a = [1, 2]; let b = [3, 4, 5]; a.concat(b).length() }
test()"#,
    )
    .expect_number(5.0);
}

/// Verifies array first after drop.
#[test]
fn test_array_first_after_drop() {
    ShapeTest::new(
        r#"function test() { [100, 200, 300, 400].drop(2).first() }
test()"#,
    )
    .expect_number(300.0);
}

/// Verifies array last after take.
#[test]
fn test_array_last_after_take() {
    ShapeTest::new(
        r#"function test() { [100, 200, 300, 400].take(2).last() }
test()"#,
    )
    .expect_number(200.0);
}

/// Verifies array reverse then take.
#[test]
fn test_array_reverse_then_take() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].reverse().take(2).last() }
test()"#,
    )
    .expect_number(4.0);
}

/// Verifies array reverse then drop.
#[test]
fn test_array_reverse_then_drop() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].reverse().drop(3).first() }
test()"#,
    )
    .expect_number(2.0);
}

/// Verifies array slice then concat.
#[test]
fn test_array_slice_then_concat() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].slice(0, 2).concat([4, 5]).length() }
test()"#,
    )
    .expect_number(4.0);
}

/// Verifies array concat then flatten.
#[test]
fn test_array_concat_then_flatten() {
    ShapeTest::new(
        r#"function test() { [[1]].concat([[2]]).flatten().length() }
test()"#,
    )
    .expect_number(2.0);
}

/// Verifies array length in condition.
#[test]
fn test_array_length_in_condition() {
    ShapeTest::new(
        r#"function test() {
            let arr = [1, 2, 3]
            if arr.length() == 3 { true } else { false }
        }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array includes after concat.
#[test]
fn test_array_includes_after_concat() {
    ShapeTest::new(
        r#"function test() { [1, 2].concat([3, 4]).includes(3) }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array index of after reverse.
#[test]
fn test_array_index_of_after_reverse() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].reverse().indexOf(10) }
test()"#,
    )
    .expect_number(2.0);
}

/// Verifies array flatten then join.
#[test]
fn test_array_flatten_then_join() {
    ShapeTest::new(
        r#"function test() { [[1, 2], [3]].flatten().join("-") }
test()"#,
    )
    .expect_string("1-2-3");
}

/// Verifies array take then join.
#[test]
fn test_array_take_then_join() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].take(3).join(", ") }
test()"#,
    )
    .expect_string("1, 2, 3");
}

/// Verifies array drop then join.
#[test]
fn test_array_drop_then_join() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].drop(3).join(", ") }
test()"#,
    )
    .expect_string("4, 5");
}
