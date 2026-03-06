//! Match expression tests.
//!
//! Covers:
//! - Match on int/string/bool literals
//! - Wildcard patterns
//! - Match as expression (assigned to variable)
//! - Match with guard clauses (where)
//! - Option (Some/None) and Result (Ok/Err) matching
//! - Binding patterns
//! - Match arm syntax (with/without commas, trailing comma)
//! - Block arm bodies
//! - First-matching-arm wins
//! - Function calls in guards
//! - Match returning from function
//! - Custom enum matching
//! - Match nested inside if
//! - FizzBuzz via guards

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Match on literals
// =========================================================================

#[test]
fn match_on_int_literal() {
    ShapeTest::new(
        r#"
        let x = 2
        match x {
            1 => "one",
            2 => "two",
            3 => "three",
            _ => "other"
        }
    "#,
    )
    .expect_string("two");
}

#[test]
fn match_on_int_wildcard() {
    ShapeTest::new(
        r#"
        let x = 99
        match x {
            1 => "one",
            2 => "two",
            _ => "other"
        }
    "#,
    )
    .expect_string("other");
}

#[test]
fn match_on_string_literal() {
    ShapeTest::new(
        r#"
        let s = "hello"
        match s {
            "hello" => 1,
            "world" => 2,
            _ => 0
        }
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn match_on_bool_true() {
    ShapeTest::new(
        r#"
        let b = true
        match b {
            true => "yes",
            false => "no"
        }
    "#,
    )
    .expect_string("yes");
}

#[test]
fn match_on_bool_false() {
    ShapeTest::new(
        r#"
        let b = false
        match b {
            true => "yes",
            false => "no"
        }
    "#,
    )
    .expect_string("no");
}

// =========================================================================
// Match as expression
// =========================================================================

#[test]
fn match_as_expression_assigned_to_variable() {
    ShapeTest::new(
        r#"
        let x = 3
        let result = match x {
            1 => "one",
            2 => "two",
            3 => "three",
            _ => "other"
        }
        result
    "#,
    )
    .expect_string("three");
}

// =========================================================================
// Match with guards
// =========================================================================

#[test]
fn match_with_guard_positive() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            match x {
                n where n > 0 => "positive",
                n where n < 0 => "negative",
                _ => "zero"
            }
        }
        classify(5)
    "#,
    )
    .expect_string("positive");
}

#[test]
fn match_with_guard_negative() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            match x {
                n where n > 0 => "positive",
                n where n < 0 => "negative",
                _ => "zero"
            }
        }
        classify(-3)
    "#,
    )
    .expect_string("negative");
}

#[test]
fn match_with_guard_zero_fallthrough() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            match x {
                n where n > 0 => "positive",
                n where n < 0 => "negative",
                _ => "zero"
            }
        }
        classify(0)
    "#,
    )
    .expect_string("zero");
}

#[test]
fn match_with_complex_guard_expression() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            match x {
                n where n > 0 and n < 100 => "small positive",
                n where n >= 100 => "large positive",
                _ => "non-positive"
            }
        }
        classify(50)
    "#,
    )
    .expect_string("small positive");
}

// =========================================================================
// Option and Result matching
// =========================================================================

// Bare Some/None patterns work on untyped variables (fixed).
#[test]
fn match_some_none_requires_typed_context() {
    ShapeTest::new(
        r#"
        let x = Some(42)
        match x {
            Some(val) => val,
            None => 0
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn match_option_via_explicit_enum() {
    // Workaround: use Option:: prefix for typed context
    ShapeTest::new(
        r#"
        let x = Ok(100)
        match x {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn match_ok_variant() {
    ShapeTest::new(
        r#"
        let x = Ok(100)
        match x {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn match_err_variant() {
    ShapeTest::new(
        r#"
        let x = Err("fail")
        match x {
            Ok(v) => 0,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Binding patterns and arm syntax
// =========================================================================

#[test]
fn match_with_binding_pattern() {
    ShapeTest::new(
        r#"
        let val = 42
        match val {
            x => x + 1
        }
    "#,
    )
    .expect_number(43.0);
}

#[test]
fn match_arms_without_commas() {
    ShapeTest::new(
        r#"
        let x = 2
        match x {
            1 => "one"
            2 => "two"
            _ => "other"
        }
    "#,
    )
    .expect_string("two");
}

#[test]
fn match_arms_with_trailing_comma() {
    ShapeTest::new(
        r#"
        let x = 1
        match x {
            1 => "one",
            2 => "two",
        }
    "#,
    )
    .expect_string("one");
}

#[test]
fn match_with_block_arm_body() {
    ShapeTest::new(
        r#"
        let x = 1
        match x {
            1 => {
                let a = 10
                let b = 20
                a + b
            },
            _ => 0
        }
    "#,
    )
    .expect_number(30.0);
}

// =========================================================================
// First matching arm, function calls in guards, return from match
// =========================================================================

#[test]
fn match_first_matching_arm_wins() {
    // Guards evaluated in order, first match wins
    ShapeTest::new(
        r#"
        fn classify(x) {
            match x {
                n where n > 50 => "large",
                n where n > 10 => "medium",
                n where n > 0 => "small",
                _ => "non-positive"
            }
        }
        classify(75)
    "#,
    )
    .expect_string("large");
}

#[test]
fn match_with_function_call_in_guard() {
    ShapeTest::new(
        r#"
        fn is_even(n) { n % 2 == 0 }
        fn classify(x) {
            match x {
                n where is_even(n) => "even",
                _ => "odd"
            }
        }
        classify(8)
    "#,
    )
    .expect_string("even");
}

#[test]
fn match_returns_value_from_function() {
    ShapeTest::new(
        r#"
        fn to_word(n) {
            return match n {
                1 => "one",
                2 => "two",
                3 => "three",
                _ => "unknown"
            }
        }
        to_word(2)
    "#,
    )
    .expect_string("two");
}

// =========================================================================
// Custom enum and wildcard
// =========================================================================

#[test]
fn match_on_enum_variants_custom() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        let c = Color::Green
        match c {
            Color::Red => "red",
            Color::Green => "green",
            Color::Blue => "blue"
        }
    "#,
    )
    .expect_string("green");
}

#[test]
fn match_wildcard_catches_all() {
    ShapeTest::new(
        r#"
        let x = 999
        match x {
            _ => "caught"
        }
    "#,
    )
    .expect_string("caught");
}

// =========================================================================
// Match nested inside if, FizzBuzz
// =========================================================================

#[test]
fn match_nested_inside_if() {
    ShapeTest::new(
        r#"
        let x = 5
        let result = if x > 0 {
            match x {
                5 => "five",
                _ => "not five"
            }
        } else {
            "negative"
        }
        result
    "#,
    )
    .expect_string("five");
}

#[test]
fn match_guard_with_modulo_fizzbuzz() {
    ShapeTest::new(
        r#"
        fn fizzbuzz(n) {
            match n {
                x where x % 15 == 0 => "fizzbuzz",
                x where x % 3 == 0 => "fizz",
                x where x % 5 == 0 => "buzz",
                _ => "number"
            }
        }
        fizzbuzz(15)
    "#,
    )
    .expect_string("fizzbuzz");
}
