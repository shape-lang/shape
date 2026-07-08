//! Stress tests for array access and length operations.

use shape_test::shape_test::ShapeTest;

/// Verifies array concat empty right.
#[test]
fn test_array_concat_empty_right() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].concat([]).length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array concat both empty — bare `[]` receiver is correctly rejected at
/// compile time (strict typing cannot infer the element type).
#[test]
fn test_array_concat_both_empty() {
    ShapeTest::new(
        r#"function test() { [].concat([]).length() }
test()"#,
    )
    .expect_run_err_contains("cannot infer the element type of this empty array");
}

/// Verifies array concat strings.
#[test]
fn test_array_concat_strings() {
    ShapeTest::new(
        r#"function test() { ["a", "b"].concat(["c"]).last() }
test()"#,
    )
    .expect_string("c");
}

/// Verifies array concat preserves order.
#[test]
fn test_array_concat_preserves_order() {
    ShapeTest::new(
        r#"function test() { [10, 20].concat([30, 40])[2] }
test()"#,
    )
    .expect_number(30.0);
}

/// Verifies array take basic.
#[test]
fn test_array_take_basic() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].take(3).length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array take first value.
#[test]
fn test_array_take_first_value() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].take(2).first() }
test()"#,
    )
    .expect_number(10.0);
}

/// Verifies array take last value.
#[test]
fn test_array_take_last_value() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].take(2).last() }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array take zero.
#[test]
fn test_array_take_zero() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].take(0).length() }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array take all.
#[test]
fn test_array_take_all() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].take(3).length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array take more than length.
#[test]
fn test_array_take_more_than_length() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].take(100).length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array take from empty.
#[test]
fn test_array_take_from_empty() {
    ShapeTest::new(
        r#"function test() { [].take(5).length() }
test()"#,
    )
    .expect_run_err_contains("cannot infer the element type of this empty array");
}

/// Verifies array drop basic.
#[test]
fn test_array_drop_basic() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4, 5].drop(2).length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array drop first value.
#[test]
fn test_array_drop_first_value() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].drop(1).first() }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array drop zero.
#[test]
fn test_array_drop_zero() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].drop(0).length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array drop all.
#[test]
fn test_array_drop_all() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].drop(3).length() }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array drop more than length.
#[test]
fn test_array_drop_more_than_length() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].drop(100).length() }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array skip alias.
#[test]
fn test_array_skip_alias() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3, 4].skip(2).first() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array drop from empty.
#[test]
fn test_array_drop_from_empty() {
    ShapeTest::new(
        r#"function test() { [].drop(3).length() }
test()"#,
    )
    .expect_run_err_contains("cannot infer the element type of this empty array");
}

/// Verifies array includes found.
#[test]
fn test_array_includes_found() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].includes(2) }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array includes not found.
#[test]
fn test_array_includes_not_found() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].includes(5) }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies array includes first element.
#[test]
fn test_array_includes_first_element() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].includes(10) }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array includes last element.
#[test]
fn test_array_includes_last_element() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].includes(30) }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array includes empty.
#[test]
fn test_array_includes_empty() {
    ShapeTest::new(
        r#"function test() { [].includes(1) }
test()"#,
    )
    .expect_run_err_contains("cannot infer the element type of this empty array");
}

/// Verifies array includes string.
#[test]
fn test_array_includes_string() {
    ShapeTest::new(
        r#"function test() { ["a", "b", "c"].includes("b") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array includes string not found.
#[test]
fn test_array_includes_string_not_found() {
    ShapeTest::new(
        r#"function test() { ["a", "b", "c"].includes("z") }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies array includes bool.
#[test]
fn test_array_includes_bool() {
    ShapeTest::new(
        r#"function test() { [true, false].includes(true) }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies array index of found.
#[test]
fn test_array_index_of_found() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].indexOf(20) }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies array index of first.
#[test]
fn test_array_index_of_first() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].indexOf(10) }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies array index of last.
#[test]
fn test_array_index_of_last() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].indexOf(30) }
test()"#,
    )
    .expect_number(2.0);
}

/// Verifies array index of not found.
#[test]
fn test_array_index_of_not_found() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].indexOf(99) }
test()"#,
    )
    .expect_number(-1.0);
}

/// Verifies array index of empty.
#[test]
fn test_array_index_of_empty() {
    ShapeTest::new(
        r#"function test() { [].indexOf(1) }
test()"#,
    )
    .expect_run_err_contains("cannot infer the element type of this empty array");
}

/// Verifies array index of first occurrence.
#[test]
fn test_array_index_of_first_occurrence() {
    ShapeTest::new(
        r#"function test() { [1, 2, 1, 2].indexOf(2) }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies array index of string.
#[test]
fn test_array_index_of_string() {
    ShapeTest::new(
        r#"function test() { ["x", "y", "z"].indexOf("y") }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies array flatten basic.
#[test]
fn test_array_flatten_basic() {
    ShapeTest::new(
        r#"function test() { [[1, 2], [3, 4]].flatten().length() }
test()"#,
    )
    .expect_number(4.0);
}

/// Verifies array flatten first value.
#[test]
fn test_array_flatten_first_value() {
    ShapeTest::new(
        r#"function test() { [[10, 20], [30, 40]].flatten().first() }
test()"#,
    )
    .expect_number(10.0);
}

/// Verifies array flatten last value.
#[test]
fn test_array_flatten_last_value() {
    ShapeTest::new(
        r#"function test() { [[10, 20], [30, 40]].flatten().last() }
test()"#,
    )
    .expect_number(40.0);
}

/// Verifies array flatten already flat.
#[test]
fn test_array_flatten_already_flat() {
    // Shape's strict flatten signature removes one nested array level and
    // returns the receiver element type. For `Array<int>`, that result is
    // `int`, so `.length()` is a static error rather than a no-op flatten.
    ShapeTest::new(
        r#"function test() { [1, 2, 3].flatten().length() }
test()"#,
    )
    .expect_run_err_contains("Method 'length' not found on type 'int'");
}

/// Verifies array flatten mixed nested.
#[test]
fn test_array_flatten_mixed_nested() {
    ShapeTest::new(
        r#"function test() { [[1, 2], [3]].flatten().length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array flatten empty inner.
#[test]
fn test_array_flatten_empty_inner() {
    ShapeTest::new(
        r#"function test() { [[], [1], []].flatten().length() }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies array flatten empty array.
#[test]
fn test_array_flatten_empty_array() {
    ShapeTest::new(
        r#"function test() { [].flatten().length() }
test()"#,
    )
    .expect_run_err_contains("cannot infer the element type of this empty array");
}

/// Verifies array flatten three nested.
#[test]
fn test_array_flatten_three_nested() {
    ShapeTest::new(
        r#"function test() { [[1], [2], [3]].flatten().length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array join with comma.
#[test]
fn test_array_join_with_comma() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].join(", ") }
test()"#,
    )
    .expect_string("1, 2, 3");
}

/// Verifies array join with dash.
#[test]
fn test_array_join_with_dash() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].join("-") }
test()"#,
    )
    .expect_string("1-2-3");
}

/// Verifies array join empty separator.
#[test]
fn test_array_join_empty_separator() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].join("") }
test()"#,
    )
    .expect_string("123");
}

/// Verifies array join single element.
#[test]
fn test_array_join_single_element() {
    ShapeTest::new(
        r#"function test() { [42].join(", ") }
test()"#,
    )
    .expect_string("42");
}

/// Verifies array join strings.
#[test]
fn test_array_join_strings() {
    ShapeTest::new(
        r#"function test() { ["a", "b", "c"].join(" ") }
test()"#,
    )
    .expect_string("a b c");
}

/// Verifies array join no separator uses comma.
#[test]
fn test_array_join_no_separator_uses_comma() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].join() }
test()"#,
    )
    .expect_string("1,2,3");
}

/// Verifies array push basic.
#[test]
fn test_array_push_basic() {
    ShapeTest::new(
        r#"function test() { let mut a = [1, 2]; a = a.push(3); a.length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array push value preserved.
#[test]
fn test_array_push_value_preserved() {
    ShapeTest::new(
        r#"function test() { let mut a = [1, 2]; a = a.push(3); a.last() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array push to empty.
#[test]
fn test_array_push_to_empty() {
    ShapeTest::new(
        r#"function test() { let mut a = []; a = a.push(42); a.first() }
test()"#,
    )
    .expect_number(42.0);
}

/// Verifies array push multiple.
#[test]
fn test_array_push_multiple() {
    ShapeTest::new(
        r#"function test() {
            let mut a = []
            a = a.push(1)
            a = a.push(2)
            a = a.push(3)
            a.length()
        }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array push preserves existing.
#[test]
fn test_array_push_preserves_existing() {
    ShapeTest::new(
        r#"function test() { let mut a = [10, 20]; a = a.push(30); a[0] }
test()"#,
    )
    .expect_number(10.0);
}

/// R1 empty-array-push let-gen (2026-06-14): a bare empty-array
/// accumulator self-push at MODULE scope (no enclosing function) must read
/// back the pushed value. The prior prototype routed the bytecode but the
/// self-push array module-binding slot read None and SIGSEGV'd (exit 139);
/// this test pins that the typed-array allocator is patched AFTER the
/// element type resolves so the module-binding slot is constructed with the
/// right kind. No `function` wrapper — top-level script context.
#[test]
fn test_array_push_to_empty_module_scope() {
    ShapeTest::new(
        r#"let mut a = []
a = a.push(42)
a.first()"#,
    )
    .expect_number(42.0);
}

/// R1 empty-array-push let-gen (2026-06-14): a loop accumulator built via
/// `a = a.push(x * x)` over a bare empty array must yield the squares. The
/// loop-body assignment compiles via the `BlockItem::Assignment` self-push
/// path; the accumulator finalizer resolves the element kind and patches the
/// placeholder allocator before the typed push fires.
#[test]
fn test_array_push_to_empty_loop_accumulator_squares() {
    ShapeTest::new(
        r#"let xs = [1, 2, 3]
let mut a = []
for x in xs {
    a = a.push(x * x)
}
a.last()"#,
    )
    .expect_number(9.0);
}

/// Verifies array in function return.
#[test]
fn test_array_in_function_return() {
    ShapeTest::new(
        r#"function test() { function make() { [1, 2, 3] } make().length() }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies array in if condition.
#[test]
fn test_array_in_if_condition() {
    ShapeTest::new(
        r#"function test() {
            let a = [1, 2, 3]
            if a.length() > 2 { "big" } else { "small" }
        }
test()"#,
    )
    .expect_string("big");
}

/// Verifies array built in loop.
#[test]
fn test_array_built_in_loop() {
    ShapeTest::new(
        r#"function test() {
            let mut a = []
            let mut i = 0
            while i < 5 {
                a = a.push(i)
                i = i + 1
            }
            a.length()
        }
test()"#,
    )
    .expect_number(5.0);
}

/// Verifies array built in loop values.
#[test]
fn test_array_built_in_loop_values() {
    ShapeTest::new(
        r#"function test() {
            let mut a = []
            let mut i = 0
            while i < 3 {
                a = a.push(i * 10)
                i = i + 1
            }
            a[2]
        }
test()"#,
    )
    .expect_number(20.0);
}

/// Verifies array for in loop.
#[test]
fn test_array_for_in_loop() {
    ShapeTest::new(
        r#"function test() {
            let mut sum = 0
            for x in [10, 20, 30] {
                sum = sum + x
            }
            sum
        }
test()"#,
    )
    .expect_number(60.0);
}

/// Verifies array chain reverse first.
#[test]
fn test_array_chain_reverse_first() {
    ShapeTest::new(
        r#"function test() { [1, 2, 3].reverse().first() }
test()"#,
    )
    .expect_number(3.0);
}

// ─────────────────────────────────────────────────────────────────────────
// `.get(i: int) -> Option<T>` — bounds-safe accessor (wave7/collection-get,
// book C4). Some(elem) when 0 <= i < len, None otherwise (i < 0 OR i >= len).
// Each case is asserted in BOTH the interpreter (default) and JIT
// (`.with_jit()`) modes, since the method is delegated to the VM handler
// from the JIT dispatch shell.
// ─────────────────────────────────────────────────────────────────────────

/// `.get(i)` in-bounds yields `Some(elem)` — unwrapped via `match`.
#[test]
fn test_array_get_in_bounds_some() {
    ShapeTest::new(
        r#"function test() { match [10, 20, 30].get(1) { Some(v) => v, None => 0 - 1 } }
test()"#,
    )
    .expect_number(20.0);
}

#[test]
fn test_array_get_in_bounds_some_jit() {
    ShapeTest::new(
        r#"function test() { match [10, 20, 30].get(1) { Some(v) => v, None => 0 - 1 } }
test()"#,
    )
    .with_jit()
    .expect_number(20.0);
}

/// `.get(i)` past the end (i >= len) yields `None`.
#[test]
fn test_array_get_out_of_bounds_high_none() {
    ShapeTest::new(
        r#"function test() { match [10, 20, 30].get(5) { Some(v) => v, None => 0 - 1 } }
test()"#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_array_get_out_of_bounds_high_none_jit() {
    ShapeTest::new(
        r#"function test() { match [10, 20, 30].get(5) { Some(v) => v, None => 0 - 1 } }
test()"#,
    )
    .with_jit()
    .expect_number(-1.0);
}

/// `.get(i)` with a negative index yields `None` (the chosen OOB rule).
#[test]
fn test_array_get_negative_index_none() {
    ShapeTest::new(
        r#"function test() { match [10, 20, 30].get(0 - 1) { Some(v) => v, None => 0 - 1 } }
test()"#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_array_get_negative_index_none_jit() {
    ShapeTest::new(
        r#"function test() { match [10, 20, 30].get(0 - 1) { Some(v) => v, None => 0 - 1 } }
test()"#,
    )
    .with_jit()
    .expect_number(-1.0);
}

/// Boundary index 0 and the last valid index both resolve to `Some`.
#[test]
fn test_array_get_boundaries() {
    ShapeTest::new(
        r#"function test() {
    let a = [10, 20, 30]
    let lo = match a.get(0) { Some(v) => v, None => 0 - 1 }
    let hi = match a.get(2) { Some(v) => v, None => 0 - 1 }
    lo + hi
}
test()"#,
    )
    .expect_number(40.0);
}

/// `.get(i) == Some(x)` — the returned carrier compares equal to a `Some`
/// value (heap Option-TypedObject equality). The `Some(20)` is bound through
/// an annotated `Option<int>` local because a bare `Some(x)` literal as a
/// direct `==` operand infers as `unknown` inside a strict `function` body
/// (a pre-existing `Some`-literal inference gap, unrelated to `.get`).
#[test]
fn test_array_get_eq_some_value() {
    ShapeTest::new(
        r#"function test() {
    let e: Option<int> = Some(20)
    [10, 20, 30].get(1) == e
}
test()"#,
    )
    .expect_bool(true);
}

/// `.get(i) == None` — the OOB result compares equal to the `None` literal.
#[test]
fn test_array_get_eq_none_literal() {
    ShapeTest::new(
        r#"function test() { [10, 20, 30].get(5) == None }
test()"#,
    )
    .expect_bool(true);
}

/// Kind-generic over the element type: string arrays return `Some(string)`.
#[test]
fn test_array_get_string_element() {
    ShapeTest::new(
        r#"function test() { match ["a", "b", "c"].get(2) { Some(v) => v, None => "none" } }
test()"#,
    )
    .expect_string("c");
}
