//! Stress tests for literal patterns (int, string, bool), wildcard/identifier patterns,
//! and match-as-expression usage.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 1: Literal Int Patterns (tests 1-12)
// =============================================================================

/// Matches int zero.
#[test]
fn t01_match_int_zero() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 0
            return match x {
                0 => 100,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(100.0);
}

/// Matches int one.
#[test]
fn t02_match_int_one() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 1
            return match x {
                0 => 0,
                1 => 100,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(100.0);
}

/// Matches negative int.
#[test]
fn t03_match_int_negative() {
    ShapeTest::new(
        r#"
        function test() {
            let x = -1
            return match x {
                -1 => 42,
                0 => 0,
                1 => 1,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Matches large int value.
#[test]
fn t04_match_int_large_value() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 999999
            return match x {
                0 => 0,
                999999 => 77,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(77.0);
}

/// Falls through to wildcard when no literal matches.
#[test]
fn t05_match_int_fallthrough_to_wildcard() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 42
            return match x {
                0 => 0,
                1 => 1,
                2 => 2,
                _ => 99
            }
        }
        test()
    "#,
    )
    .expect_number(99.0);
}

/// First arm wins when multiple arms could match.
#[test]
fn t06_match_int_first_arm_wins() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 5
            return match x {
                5 => 100,
                5 => 200,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(100.0);
}

/// Matches among many arms.
#[test]
fn t07_match_int_many_arms() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 7
            return match x {
                0 => 0,
                1 => 10,
                2 => 20,
                3 => 30,
                4 => 40,
                5 => 50,
                6 => 60,
                7 => 70,
                8 => 80,
                9 => 90,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(70.0);
}

/// Matches the last literal arm.
#[test]
fn t08_match_int_last_literal_arm() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 9
            return match x {
                0 => 0,
                1 => 10,
                2 => 20,
                3 => 30,
                4 => 40,
                5 => 50,
                6 => 60,
                7 => 70,
                8 => 80,
                9 => 90,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(90.0);
}

/// Matches large negative int.
#[test]
fn t09_match_int_negative_large() {
    ShapeTest::new(
        r#"
        function test() {
            let x = -1000
            return match x {
                -1000 => 1,
                0 => 2,
                1000 => 3,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Matches on expression result.
#[test]
fn t10_match_on_expression_result() {
    ShapeTest::new(
        r#"
        function test() {
            return match 2 + 3 {
                4 => 0,
                5 => 1,
                6 => 2,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Matches on function return value.
#[test]
fn t11_match_on_function_return() {
    ShapeTest::new(
        r#"
        function get_val() { return 3 }
        function test() {
            let v = get_val()
            return match v {
                1 => 10,
                2 => 20,
                3 => 30,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(30.0);
}

/// Two arms only, wildcard matches.
#[test]
fn t12_match_int_two_arms_only() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 1
            return match x {
                0 => 10,
                _ => 20
            }
        }
        test()
    "#,
    )
    .expect_number(20.0);
}

// =============================================================================
// SECTION 2: Literal String Patterns (tests 13-20)
// =============================================================================

/// Matches basic string.
#[test]
fn t13_match_string_basic() {
    ShapeTest::new(
        r#"
        function test() {
            let name = "Alice"
            return match name {
                "Alice" => 1,
                "Bob" => 2,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Matches second string arm.
#[test]
fn t14_match_string_second_arm() {
    ShapeTest::new(
        r#"
        function test() {
            let name = "Bob"
            return match name {
                "Alice" => 1,
                "Bob" => 2,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// String wildcard fallback.
#[test]
fn t15_match_string_wildcard_fallback() {
    ShapeTest::new(
        r#"
        function test() {
            let name = "Charlie"
            return match name {
                "Alice" => 1,
                "Bob" => 2,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(0.0);
}

/// Matches empty string.
#[test]
fn t16_match_empty_string() {
    ShapeTest::new(
        r#"
        function test() {
            let s = ""
            return match s {
                "" => 1,
                "x" => 2,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Matches among many string arms.
#[test]
fn t17_match_string_many_arms() {
    ShapeTest::new(
        r#"
        function test() {
            let op = "mul"
            return match op {
                "add" => 1,
                "sub" => 2,
                "mul" => 3,
                "div" => 4,
                "mod" => 5,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

/// Match string returning string.
#[test]
fn t18_match_string_return_string() {
    ShapeTest::new(
        r#"
        function test() {
            let x = "yes"
            return match x {
                "yes" => "affirmative",
                "no" => "negative",
                _ => "unknown"
            }
        }
        test()
    "#,
    )
    .expect_string("affirmative");
}

/// Matches string with spaces.
#[test]
fn t19_match_string_with_spaces() {
    ShapeTest::new(
        r#"
        function test() {
            let s = "hello world"
            return match s {
                "hello world" => 1,
                "goodbye" => 2,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Matches string with special characters.
#[test]
fn t20_match_string_special_chars() {
    ShapeTest::new(
        r#"
        function test() {
            let s = "hello\nworld"
            return match s {
                "hello\nworld" => 1,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

// =============================================================================
// SECTION 3: Boolean Patterns (tests 21-26)
// =============================================================================

/// Matches bool true.
#[test]
fn t21_match_bool_true() {
    ShapeTest::new(
        r#"
        function test() {
            let flag = true
            return match flag {
                true => 1,
                false => 0,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Matches bool false.
#[test]
fn t22_match_bool_false() {
    ShapeTest::new(
        r#"
        function test() {
            let flag = false
            return match flag {
                true => 1,
                false => 0,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(0.0);
}

/// Matches bool from expression.
#[test]
fn t23_match_bool_expression() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 5
            let flag = x > 3
            return match flag {
                true => 100,
                false => 0,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(100.0);
}

/// Bool match without wildcard.
#[test]
fn t24_match_bool_without_wildcard() {
    ShapeTest::new(
        r#"
        function test() {
            let flag = true
            return match flag {
                true => 42,
                false => 0
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// False arm matches.
#[test]
fn t25_match_bool_false_arm_only() {
    ShapeTest::new(
        r#"
        function test() {
            let flag = false
            return match flag {
                true => 42,
                false => 99
            }
        }
        test()
    "#,
    )
    .expect_number(99.0);
}

/// Bool match returns string.
#[test]
fn t26_match_bool_return_string() {
    ShapeTest::new(
        r#"
        function test() {
            let flag = true
            return match flag {
                true => "yes",
                false => "no"
            }
        }
        test()
    "#,
    )
    .expect_string("yes");
}

// =============================================================================
// SECTION 4: Wildcard & Identifier Patterns (tests 27-34)
// =============================================================================

/// Wildcard catch-all.
#[test]
fn t27_wildcard_catch_all() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 999
            return match x {
                _ => 42
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Identifier binding in match.
#[test]
fn t28_identifier_binding() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 10
            return match x {
                n => n + 1
            }
        }
        test()
    "#,
    )
    .expect_number(11.0);
}

/// Identifier binding after literal arm.
#[test]
fn t29_identifier_binding_after_literal() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 5
            return match x {
                0 => 0,
                n => n * 2
            }
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Identifier used in computation.
#[test]
fn t30_identifier_binding_used_in_computation() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 7
            return match x {
                0 => 100,
                1 => 200,
                val => val * val
            }
        }
        test()
    "#,
    )
    .expect_number(49.0);
}

/// Wildcard discards value.
#[test]
fn t31_wildcard_discards_value() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 42
            return match x {
                0 => 0,
                _ => 99
            }
        }
        test()
    "#,
    )
    .expect_number(99.0);
}

/// Identifier pattern shadows outer variable.
#[test]
fn t32_identifier_shadows_outer() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 10
            let n = 999
            let result = match x {
                n => n + 5
            }
            return result
        }
        test()
    "#,
    )
    .expect_number(15.0);
}

/// Single identifier arm always matches.
#[test]
fn t33_multiple_identifier_arms() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 42
            return match x {
                a => a + 1
            }
        }
        test()
    "#,
    )
    .expect_number(43.0);
}

/// Wildcard with block body.
#[test]
fn t34_wildcard_with_block_body() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 5
            return match x {
                0 => 0,
                _ => {
                    let a = 10
                    let b = 20
                    a + b
                }
            }
        }
        test()
    "#,
    )
    .expect_number(30.0);
}

// =============================================================================
// SECTION 5: Match as Expression (tests 35-42)
// =============================================================================

/// Match expression in let binding.
#[test]
fn t35_match_as_expression_in_let() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 2
            let result = match x {
                1 => 10,
                2 => 20,
                _ => 0
            }
            return result
        }
        test()
    "#,
    )
    .expect_number(20.0);
}

/// Match expression in return.
#[test]
fn t36_match_as_expression_in_return() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 3
            return match x {
                1 => "one",
                2 => "two",
                3 => "three",
                _ => "other"
            }
        }
        test()
    "#,
    )
    .expect_string("three");
}

/// Match in arithmetic expression.
#[test]
fn t37_match_in_arithmetic() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 2
            let y = 10 + match x {
                1 => 100,
                2 => 200,
                _ => 0
            }
            return y
        }
        test()
    "#,
    )
    .expect_number(210.0);
}

/// Nested match expressions.
#[test]
fn t38_nested_match_expressions() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 1
            let y = 2
            return match x {
                1 => match y {
                    1 => 10,
                    2 => 20,
                    _ => 0
                },
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(20.0);
}

/// Match result passed to function.
#[test]
fn t39_match_result_passed_to_function() {
    ShapeTest::new(
        r#"
        function double(n) { return n * 2 }
        function test() {
            let x = 3
            return double(match x {
                1 => 10,
                2 => 20,
                3 => 30,
                _ => 0
            })
        }
        test()
    "#,
    )
    .expect_number(60.0);
}

/// Match with block arms.
#[test]
fn t40_match_with_block_arms() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 2
            return match x {
                1 => {
                    let a = 1
                    let b = 2
                    a + b
                },
                2 => {
                    let a = 10
                    let b = 20
                    a * b
                },
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(200.0);
}

/// Match expression chained through functions.
#[test]
fn t41_match_expression_chained() {
    ShapeTest::new(
        r#"
        function classify(n) {
            return match n {
                0 => "zero",
                1 => "one",
                _ => "many"
            }
        }
        function test() {
            let r1 = classify(0)
            let r2 = classify(1)
            let r3 = classify(99)
            return r3
        }
        test()
    "#,
    )
    .expect_string("many");
}

/// Match in loop.
#[test]
fn t42_match_in_loop() {
    ShapeTest::new(
        r#"
        function test() {
            let mut total = 0
            for i in range(5) {
                total = total + match i {
                    0 => 10,
                    1 => 20,
                    2 => 30,
                    _ => 1
                }
            }
            return total
        }
        test()
    "#,
    )
    .expect_number(62.0);
}
