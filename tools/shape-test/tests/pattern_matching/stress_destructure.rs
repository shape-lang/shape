//! Stress tests for array/object destructuring, constructor patterns, let destructuring,
//! and function parameter destructuring in match.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 7: Array Destructuring in match (tests 53-60)
// =============================================================================

/// Match array two elements.
#[test]
fn t53_match_array_two_elements() {
    ShapeTest::new(
        r#"
        function test() {
            return match [1, 2] {
                [a, b] => a + b,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

/// Match array three elements.
#[test]
fn t54_match_array_three_elements() {
    ShapeTest::new(
        r#"
        function test() {
            return match [10, 20, 30] {
                [a, b, c] => a + b + c,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(60.0);
}

/// Match array with literal element.
#[test]
fn t55_match_array_with_literal_element() {
    ShapeTest::new(
        r#"
        function test() {
            return match [0, 42] {
                [0, x] => x,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Match array literal mismatch.
#[test]
fn t56_match_array_literal_mismatch() {
    ShapeTest::new(
        r#"
        function test() {
            return match [1, 42] {
                [0, x] => x,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(-1.0);
}

/// Match array length mismatch.
#[test]
fn t57_match_array_length_mismatch() {
    ShapeTest::new(
        r#"
        function test() {
            return match [1, 2, 3] {
                [a, b] => a + b,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(-1.0);
}

/// Match array single element.
#[test]
fn t58_match_array_single_element() {
    ShapeTest::new(
        r#"
        function test() {
            return match [99] {
                [x] => x,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(99.0);
}

/// Match array wildcard element.
#[test]
fn t59_match_array_wildcard_element() {
    ShapeTest::new(
        r#"
        function test() {
            return match [1, 2] {
                [_, b] => b,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Match array with nested computation.
#[test]
fn t60_match_array_nested_computation() {
    ShapeTest::new(
        r#"
        function test() {
            let arr = [3, 4]
            return match arr {
                [a, b] => a * b + 1,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(13.0);
}

// =============================================================================
// SECTION 8: Object Destructuring in match (tests 61-68)
// =============================================================================

/// Match object basic.
#[test]
fn t61_match_object_basic() {
    ShapeTest::new(
        r#"
        function test() {
            return match { a: 1, b: 2 } {
                { a: x, b: y } => x + y,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

/// Match object shorthand.
#[test]
fn t62_match_object_shorthand() {
    ShapeTest::new(
        r#"
        function test() {
            return match { a: 10, b: 20 } {
                { a, b } => a + b,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(30.0);
}

/// Match object with literal field.
#[test]
fn t63_match_object_with_literal_field() {
    ShapeTest::new(
        r#"
        function test() {
            return match { a: 1, b: 42 } {
                { a: 1, b: x } => x,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Match object literal mismatch.
#[test]
fn t64_match_object_literal_mismatch() {
    ShapeTest::new(
        r#"
        function test() {
            return match { a: 2, b: 42 } {
                { a: 1, b: x } => x,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(-1.0);
}

/// Match object single field.
#[test]
fn t65_match_object_single_field() {
    ShapeTest::new(
        r#"
        function test() {
            return match { x: 77 } {
                { x } => x,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(77.0);
}

/// Match object many fields.
#[test]
fn t66_match_object_many_fields() {
    ShapeTest::new(
        r#"
        function test() {
            return match { x: 1, y: 2, z: 3 } {
                { x, y, z } => x + y + z,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

/// Match object variable scrutinee.
#[test]
fn t67_match_object_variable_scrutinee() {
    ShapeTest::new(
        r#"
        function test() {
            let point = { x: 5, y: 10 }
            return match point {
                { x, y } => x * y,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(50.0);
}

/// Match object field rename.
#[test]
fn t68_match_object_field_rename() {
    ShapeTest::new(
        r#"
        function test() {
            return match { a: 42, b: 0 } {
                { a: val, b: _ } => val,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =============================================================================
// SECTION 9: Constructor Patterns - Option/Result (tests 69-80)
// =============================================================================

/// Match Option Some.
#[test]
fn t69_match_option_some() {
    ShapeTest::new(
        r#"
        function test() {
            let x = Some(42)
            return match x {
                Some(v) => v,
                None => 0
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Match Option None.
#[test]
fn t70_match_option_none() {
    ShapeTest::new(
        r#"
        function test() {
            let x = None
            return match x {
                Some(v) => v,
                None => -1
            }
        }
        test()
    "#,
    )
    .expect_number(-1.0);
}

/// Match Option qualified Some.
#[test]
fn t71_match_option_qualified() {
    ShapeTest::new(
        r#"
        function test() {
            let x = Some(99)
            return match x {
                Some(v) => v,
                None => 0
            }
        }
        test()
    "#,
    )
    .expect_number(99.0);
}

/// Match Result Ok.
#[test]
fn t72_match_result_ok() {
    ShapeTest::new(
        r#"
        function test() {
            let x = Ok(10)
            return match x {
                Ok(v) => v,
                Err(e) => -1
            }
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Match Result Err.
#[test]
fn t73_match_result_err() {
    ShapeTest::new(
        r#"
        function test() {
            let x = Err("bad")
            return match x {
                Ok(v) => 0,
                Err(e) => -1
            }
        }
        test()
    "#,
    )
    .expect_number(-1.0);
}

/// Match Result Err extract string.
#[test]
fn t74_match_result_err_extract() {
    ShapeTest::new(
        r#"
        function test() {
            let x = Err("oops")
            return match x {
                Ok(v) => "ok",
                Err(e) => e
            }
        }
        test()
    "#,
    )
    .expect_string("oops");
}

/// Match Option None returns default.
#[test]
fn t75_match_option_none_returns_default() {
    ShapeTest::new(
        r#"
        function get_or_default(opt, default_val) {
            return match opt {
                Some(v) => v,
                None => default_val
            }
        }
        function test() {
            return get_or_default(None, 42)
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Match Option Some ignores default.
#[test]
fn t76_match_option_some_ignores_default() {
    ShapeTest::new(
        r#"
        function get_or_default(opt, default_val) {
            return match opt {
                Some(v) => v,
                None => default_val
            }
        }
        function test() {
            return get_or_default(Some(10), 42)
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Match Result Ok qualified.
#[test]
fn t77_match_result_ok_qualified() {
    ShapeTest::new(
        r#"
        function test() {
            let x = Ok(77)
            return match x {
                Result::Ok(v) => v,
                Result::Err(e) => 0
            }
        }
        test()
    "#,
    )
    .expect_number(77.0);
}

/// Match nested Some.
#[test]
fn t78_match_nested_some() {
    ShapeTest::new(
        r#"
        function test() {
            let x = Some(100)
            let result = match x {
                Some(v) => v + 5,
                None => 0
            }
            return result
        }
        test()
    "#,
    )
    .expect_number(105.0);
}

/// Match Result with wildcard.
#[test]
fn t79_match_result_with_wildcard() {
    ShapeTest::new(
        r#"
        function test() {
            let x = Ok(42)
            return match x {
                Ok(v) => v,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =============================================================================
// SECTION 10: Let Destructuring (tests 81-92)
// =============================================================================

/// Let array destructure basic.
#[test]
fn t81_let_array_destructure_basic() {
    ShapeTest::new(
        r#"
        function test() {
            let [a, b] = [1, 2]
            return a + b
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

/// Let array destructure three.
#[test]
fn t82_let_array_destructure_three() {
    ShapeTest::new(
        r#"
        function test() {
            let [a, b, c] = [10, 20, 30]
            return a + b + c
        }
        test()
    "#,
    )
    .expect_number(60.0);
}

/// Let object destructure basic.
#[test]
fn t83_let_object_destructure_basic() {
    ShapeTest::new(
        r#"
        function test() {
            let { a, b } = { a: 5, b: 10 }
            return a + b
        }
        test()
    "#,
    )
    .expect_number(15.0);
}

/// Let object destructure three fields.
#[test]
fn t84_let_object_destructure_three_fields() {
    ShapeTest::new(
        r#"
        function test() {
            let { x, y, z } = { x: 1, y: 2, z: 3 }
            return x + y + z
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

/// Let object rest.
#[test]
fn t85_let_object_rest() {
    ShapeTest::new(
        r#"
        function test() {
            let { a, ...rest } = { a: 1, b: 2 }
            return rest.b
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Let array rest.
#[test]
fn t86_let_array_rest() {
    ShapeTest::new(
        r#"
        function test() {
            let [first, ...rest] = [10, 20, 30, 40]
            return first
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Let destructure from function.
#[test]
fn t87_let_destructure_from_function() {
    ShapeTest::new(
        r#"
        function make_pair() { return { x: 3, y: 4 } }
        function test() {
            let { x, y } = make_pair()
            return x + y
        }
        test()
    "#,
    )
    .expect_number(7.0);
}

/// Let array destructure single.
#[test]
fn t88_let_array_destructure_single() {
    ShapeTest::new(
        r#"
        function test() {
            let [x] = [42]
            return x
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Let object destructure string values.
#[test]
fn t89_let_object_destructure_string_values() {
    ShapeTest::new(
        r#"
        function test() {
            let { name, age } = { name: "Alice", age: 30 }
            return name
        }
        test()
    "#,
    )
    .expect_string("Alice");
}

/// Let destructure in loop body.
#[test]
fn t90_let_destructure_in_loop_body() {
    ShapeTest::new(
        r#"
        function test() {
            let items = [[1, 2], [3, 4], [5, 6]]
            var total = 0
            for item in items {
                let [a, b] = item
                total = total + a + b
            }
            return total
        }
        test()
    "#,
    )
    .expect_number(21.0);
}

/// Let nested object destructure.
#[test]
fn t91_let_nested_object_destructure() {
    ShapeTest::new(
        r#"
        function test() {
            let { point: { x, y } } = { point: { x: 5, y: 10 } }
            return x + y
        }
        test()
    "#,
    )
    .expect_number(15.0);
}

/// Let array destructure mixed types.
#[test]
fn t92_let_array_destructure_mixed_types() {
    ShapeTest::new(
        r#"
        function test() {
            let [a, b, c] = [1, "hello", true]
            return a
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

// =============================================================================
// SECTION 11: Function Param Destructuring (tests 93-100)
// =============================================================================

/// Param object destructure.
#[test]
fn t93_param_object_destructure() {
    ShapeTest::new(
        r#"
        function add({x, y}) { return x + y }
        function test() {
            return add({x: 10, y: 20})
        }
        test()
    "#,
    )
    .expect_number(30.0);
}

/// Param array destructure.
#[test]
fn t94_param_array_destructure() {
    ShapeTest::new(
        r#"
        function sum([a, b]) { return a + b }
        function test() {
            return sum([5, 15])
        }
        test()
    "#,
    )
    .expect_number(20.0);
}

/// Param nested destructure.
#[test]
fn t95_param_nested_destructure() {
    ShapeTest::new(
        r#"
        function process({point: {x, y}}) {
            return x + y
        }
        function test() {
            return process({point: {x: 5, y: 10}})
        }
        test()
    "#,
    )
    .expect_number(15.0);
}

/// Lambda object destructure.
#[test]
fn t96_lambda_object_destructure() {
    ShapeTest::new(
        r#"
        function test() {
            let add = |{x, y}| x + y
            return add({x: 7, y: 3})
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Lambda array destructure.
#[test]
fn t97_lambda_array_destructure() {
    ShapeTest::new(
        r#"
        function test() {
            let sum = |[a, b]| a + b
            return sum([100, 200])
        }
        test()
    "#,
    )
    .expect_number(300.0);
}

/// For loop object destructure.
#[test]
fn t98_for_loop_object_destructure() {
    ShapeTest::new(
        r#"
        function test() {
            let points = [{x: 1, y: 2}, {x: 3, y: 4}]
            var sum = 0
            for {x, y} in points {
                sum = sum + x + y
            }
            return sum
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Param destructure three fields.
#[test]
fn t99_param_destructure_three_fields() {
    ShapeTest::new(
        r#"
        function volume({w, h, d}) { return w * h * d }
        function test() {
            return volume({w: 2, h: 3, d: 4})
        }
        test()
    "#,
    )
    .expect_number(24.0);
}

/// Param destructure used in condition.
#[test]
fn t100_param_destructure_used_in_condition() {
    ShapeTest::new(
        r#"
        function is_origin({x, y}) {
            if (x == 0 and y == 0) { return true }
            return false
        }
        function test() {
            return is_origin({x: 0, y: 0})
        }
        test()
    "#,
    )
    .expect_bool(true);
}
