//! Stress tests for advanced pattern matching: error cases, user-defined enums,
//! top-level patterns, combined patterns, intersection decomposition, compile-time failures,
//! and edge cases.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 12: No Match / Error Cases (tests 101-106)
// =============================================================================

/// No match throws runtime error.
#[test]
fn t101_no_match_throws() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 5
            return match x {
                1 => 1,
                2 => 2,
                3 => 3
            }
        }
        test()
    "#,
    )
    .expect_run_err();
}

/// Empty array doesn't match [a, b] pattern.
#[test]
fn t102_match_empty_array_no_match() {
    ShapeTest::new(
        r#"
        function test() {
            return match [] {
                [a, b] => a + b
            }
        }
        test()
    "#,
    )
    .expect_run_err();
}

/// None matches None pattern.
#[test]
fn t103_match_null_against_none() {
    ShapeTest::new(
        r#"
        function test() {
            let x = None
            return match x {
                None => 1,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Non-null value matches Some(v) pattern.
#[test]
fn t104_match_non_null_against_some() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 42
            return match x {
                None => 0,
                Some(v) => v
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Match float literal.
#[test]
fn t105_match_float_literal() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 3.14
            return match x {
                3.14 => 1,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Match mixed literal types in arms.
#[test]
fn t106_match_mixed_literal_types() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 42
            return match x {
                "hello" => 1,
                true => 2,
                42 => 3,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

// =============================================================================
// SECTION 13: Advanced / Edge Cases (tests 107-115)
// =============================================================================

/// Match inside match.
#[test]
fn t107_match_in_match() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 1
            let y = "b"
            return match x {
                1 => match y {
                    "a" => 10,
                    "b" => 20,
                    _ => 0
                },
                2 => 100,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(20.0);
}

/// Match result used conditionally.
#[test]
fn t108_match_result_used_conditionally() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 5
            let label = match x {
                0 => "zero",
                _ => "nonzero"
            }
            if (label == "nonzero") {
                return 1
            }
            return 0
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Sequential matches.
#[test]
fn t109_match_sequential_matches() {
    ShapeTest::new(
        r#"
        function test() {
            let a = match 1 { 1 => 10, _ => 0 }
            let b = match 2 { 2 => 20, _ => 0 }
            let c = match 3 { 3 => 30, _ => 0 }
            return a + b + c
        }
        test()
    "#,
    )
    .expect_number(60.0);
}

/// Match with function call in arm.
#[test]
fn t110_match_with_function_call_in_arm() {
    ShapeTest::new(
        r#"
        function square(n) { return n * n }
        function test() {
            let x = 3
            return match x {
                0 => 0,
                n => square(n)
            }
        }
        test()
    "#,
    )
    .expect_number(9.0);
}

/// Match wildcard only.
#[test]
fn t111_match_wildcard_only() {
    ShapeTest::new(
        r#"
        function test() {
            return match "anything" {
                _ => 42
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Match string in function.
#[test]
fn t112_match_string_in_function() {
    ShapeTest::new(
        r#"
        function eval(op, a, b) {
            return match op {
                "add" => a + b,
                "sub" => a - b,
                "mul" => a * b,
                _ => 0
            }
        }
        function test() {
            return eval("mul", 6, 7)
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Match recursive function (fibonacci).
#[test]
fn t115_match_recursive_function() {
    ShapeTest::new(
        r#"
        function fib(n) {
            return match n {
                0 => 0,
                1 => 1,
                x => fib(x - 1) + fib(x - 2)
            }
        }
        function test() {
            return fib(10)
        }
        test()
    "#,
    )
    .expect_number(55.0);
}

// =============================================================================
// SECTION 14: Enum Patterns with User-Defined Enums (tests 116-120)
// =============================================================================

/// Match user enum basic.
#[test]
fn t116_match_user_enum_basic() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        function test() {
            let c = Color::Red
            return match c {
                Color::Red => 1,
                Color::Green => 2,
                Color::Blue => 3
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Match user enum second variant.
#[test]
fn t117_match_user_enum_second_variant() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        function test() {
            let c = Color::Green
            return match c {
                Color::Red => 1,
                Color::Green => 2,
                Color::Blue => 3
            }
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Match user enum third variant.
#[test]
fn t118_match_user_enum_third_variant() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        function test() {
            let c = Color::Blue
            return match c {
                Color::Red => 1,
                Color::Green => 2,
                Color::Blue => 3
            }
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

/// Match user enum with payload.
#[test]
fn t119_match_user_enum_with_payload() {
    ShapeTest::new(
        r#"
        enum Shape {
            Circle(number),
            Rect(number, number)
        }
        function test() {
            let s = Shape::Circle(5.0)
            return match s {
                Shape::Circle(r) => r,
                Shape::Rect(w, h) => w * h
            }
        }
        test()
    "#,
    )
    .expect_number(5.0);
}

/// Match user enum rect variant.
#[test]
fn t120_match_user_enum_rect_variant() {
    ShapeTest::new(
        r#"
        enum Shape {
            Circle(number),
            Rect(number, number)
        }
        function test() {
            let s = Shape::Rect(3.0, 4.0)
            return match s {
                Shape::Circle(r) => r,
                Shape::Rect(w, h) => w * h
            }
        }
        test()
    "#,
    )
    .expect_number(12.0);
}

// =============================================================================
// SECTION 15: Top-Level Patterns (tests 121-125)
// =============================================================================

/// Top-level let array destructure.
#[test]
fn t121_top_level_let_array_destructure() {
    ShapeTest::new(
        r#"
        let [a, b] = [10, 20]
        a + b
    "#,
    )
    .expect_number(30.0);
}

/// Top-level let object destructure.
#[test]
fn t122_top_level_let_object_destructure() {
    ShapeTest::new(
        r#"
        let { x, y } = { x: 100, y: 200 }
        x + y
    "#,
    )
    .expect_number(300.0);
}

/// Top-level match.
#[test]
fn t123_top_level_match() {
    ShapeTest::new(
        r#"
        let x = 3
        match x {
            1 => 10,
            2 => 20,
            3 => 30,
            _ => 0
        }
    "#,
    )
    .expect_number(30.0);
}

/// Top-level match string.
#[test]
fn t124_top_level_match_string() {
    ShapeTest::new(
        r#"
        let s = "hi"
        match s {
            "hi" => 1,
            "bye" => 2,
            _ => 0
        }
    "#,
    )
    .expect_number(1.0);
}

/// Top-level object rest destructure.
#[test]
fn t125_top_level_object_rest_destructure() {
    ShapeTest::new(
        r#"
        let { a, ...rest } = { a: 1, b: 2, c: 3 }
        a
    "#,
    )
    .expect_number(1.0);
}

// =============================================================================
// SECTION 16: Combined Pattern Types (tests 126-130)
// =============================================================================

/// Match array then wildcard.
#[test]
fn t129_match_array_then_wildcard() {
    ShapeTest::new(
        r#"
        function test() {
            let data = [1, 2]
            return match data {
                [0, _] => "starts with zero",
                [_, 0] => "ends with zero",
                [a, b] => "other",
                _ => "unknown"
            }
        }
        test()
    "#,
    )
    .expect_string("other");
}

/// Match array starts with zero.
#[test]
fn t130_match_array_starts_with_zero() {
    ShapeTest::new(
        r#"
        function test() {
            let data = [0, 5]
            return match data {
                [0, x] => x,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(5.0);
}

// =============================================================================
// SECTION 18: Compile-Time Failures (tests 133-136)
// =============================================================================

/// Object destructure unknown field fails at compile time.
#[test]
fn t133_object_destructure_unknown_field_fails() {
    ShapeTest::new(
        r#"
        function test() {
            let { x, y, z } = { x: 1, y: 2 }
            return x
        }
    "#,
    )
    .expect_run_err();
}

/// Match object unknown field fails at compile time.
#[test]
fn t134_match_object_unknown_field_fails() {
    ShapeTest::new(
        r#"
        function test() {
            return match { a: 1 } {
                { a, b } => a,
                _ => 0
            }
        }
    "#,
    )
    .expect_run_err();
}

/// Match with no arms fails.
#[test]
fn t135_parse_match_no_arms() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 1
            return match x { }
        }
        test()
    "#,
    )
    .expect_run_err();
}

/// Arm bodies with blocks using semicolons.
#[test]
fn t136_match_semicolons_in_block_arms() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 1
            return match x {
                1 => {
                    let a = 10;
                    let b = 20;
                    a + b
                },
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(30.0);
}

// =============================================================================
// SECTION 19: Edge Cases & Regression-Style (tests 137-145)
// =============================================================================

/// Match with var mutation in arm.
#[test]
fn t137_match_with_var_mutation_in_arm() {
    ShapeTest::new(
        r#"
        function test() {
            var acc = 0
            let x = 2
            match x {
                1 => { acc = acc + 10 },
                2 => { acc = acc + 20 },
                _ => {}
            }
            return acc
        }
        test()
    "#,
    )
    .expect_number(20.0);
}

/// Match multiple times same var.
#[test]
fn t138_match_multiple_times_same_var() {
    ShapeTest::new(
        r#"
        function test() {
            var x = 1
            let r1 = match x { 1 => 10, _ => 0 }
            x = 2
            let r2 = match x { 2 => 20, _ => 0 }
            x = 3
            let r3 = match x { 3 => 30, _ => 0 }
            return r1 + r2 + r3
        }
        test()
    "#,
    )
    .expect_number(60.0);
}

/// Match trailing comma.
#[test]
fn t139_match_trailing_comma() {
    ShapeTest::new(
        r#"
        function test() {
            return match 1 {
                1 => 10,
                _ => 0,
            }
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Match arm returns bool.
#[test]
fn t140_match_arm_returns_bool() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 5
            return match x {
                5 => true,
                _ => false
            }
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Match arm returns array, check length.
#[test]
fn t141_match_arm_returns_array() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 1
            let arr = match x {
                1 => [10, 20],
                _ => [0, 0]
            }
            return arr.length()
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// For loop array destructure.
#[test]
fn t144_for_loop_array_destructure() {
    ShapeTest::new(
        r#"
        function test() {
            let pairs = [[1, 2], [3, 4], [5, 6]]
            var sum = 0
            for [a, b] in pairs {
                sum = sum + a * b
            }
            return sum
        }
        test()
    "#,
    )
    .expect_number(44.0);
}

/// Match fibonacci iterative.
#[test]
fn t145_match_fibonacci_iterative() {
    ShapeTest::new(
        r#"
        function test() {
            var n = 10
            return match n {
                0 => 0,
                1 => 1,
                _ => {
                    var a = 0
                    var b = 1
                    var i = 2
                    while (i <= n) {
                        let temp = a + b
                        a = b
                        b = temp
                        i = i + 1
                    }
                    b
                }
            }
        }
        test()
    "#,
    )
    .expect_number(55.0);
}
