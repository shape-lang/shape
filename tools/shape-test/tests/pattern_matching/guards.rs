//! Guard clause tests for pattern matching.
//!
//! Tests cover:
//! - Nested match expressions
//! - No matching arm (runtime error)
//! - Overlapping patterns (first-match-wins)
//! - Range-like guards
//! - Complex guard expressions
//! - Bound variables in arm bodies
//! - Match as implicit return

use shape_test::shape_test::ShapeTest;

// ============================================================================
// 8. Nested match expressions
// ============================================================================

#[test]
fn pm_08_nested_match() {
    let code = r#"
function classify(x, y) {
    return match x {
        a where a > 0 => match y {
            b where b > 0 => "both positive",
            _ => "only x positive"
        },
        _ => "x not positive"
    }
}
classify(1, 1)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("both positive");
}

#[test]
fn pm_08_nested_match_inner_wildcard() {
    let code = r#"
function classify(x, y) {
    return match x {
        a where a > 0 => match y {
            b where b > 0 => "both positive",
            _ => "only x positive"
        },
        _ => "x not positive"
    }
}
classify(1, -1)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("only x positive");
}

#[test]
fn pm_08_nested_match_outer_wildcard() {
    let code = r#"
function classify(x, y) {
    return match x {
        a where a > 0 => match y {
            b where b > 0 => "both positive",
            _ => "only x positive"
        },
        _ => "x not positive"
    }
}
classify(-1, 5)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("x not positive");
}

// ============================================================================
// 9. No matching arm (runtime error)
// ============================================================================

#[test]
fn pm_09_no_match_arm_runtime_error() {
    let code = r#"
function check(n) {
    return match n {
        x where x == 1 => "one",
        x where x == 2 => "two"
    }
}
check(99)
"#;
    ShapeTest::new(code).expect_run_err();
}

// ============================================================================
// 10. Overlapping patterns (first match wins)
// ============================================================================

#[test]
fn pm_10_first_match_wins() {
    let code = r#"
function classify(n) {
    return match n {
        x where x > 10 => "large",
        x where x > 0 => "small",
        _ => "non-positive"
    }
}
classify(15)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("large");
}

#[test]
fn pm_10_first_match_wins_second_arm() {
    let code = r#"
function classify(n) {
    return match n {
        x where x > 10 => "large",
        x where x > 0 => "small",
        _ => "non-positive"
    }
}
classify(5)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("small");
}

#[test]
fn pm_10_overlapping_multiple_all_match_first_wins() {
    let code = r#"
function classify(n) {
    return match n {
        x where x > 50 => "large",
        x where x > 10 => "medium",
        x where x > 0 => "small",
        _ => "non-positive"
    }
}
classify(100)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("large");
}

// ============================================================================
// 11. Range-like guards
// ============================================================================

#[test]
fn pm_11_range_guard_in_range() {
    let code = r#"
function in_range(n) {
    return match n {
        x where x >= 10 and x <= 20 => "in range",
        _ => "out of range"
    }
}
in_range(15)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("in range");
}

#[test]
fn pm_11_range_guard_below_range() {
    let code = r#"
function in_range(n) {
    return match n {
        x where x >= 10 and x <= 20 => "in range",
        _ => "out of range"
    }
}
in_range(5)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("out of range");
}

#[test]
fn pm_11_range_guard_above_range() {
    let code = r#"
function in_range(n) {
    return match n {
        x where x >= 10 and x <= 20 => "in range",
        _ => "out of range"
    }
}
in_range(25)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("out of range");
}

#[test]
fn pm_11_range_guard_boundary_low() {
    let code = r#"
function in_range(n) {
    return match n {
        x where x >= 10 and x <= 20 => "in range",
        _ => "out of range"
    }
}
in_range(10)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("in range");
}

#[test]
fn pm_11_range_guard_boundary_high() {
    let code = r#"
function in_range(n) {
    return match n {
        x where x >= 10 and x <= 20 => "in range",
        _ => "out of range"
    }
}
in_range(20)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("in range");
}

// ============================================================================
// 12. Complex guard expressions
// ============================================================================

#[test]
fn pm_12_modulo_guard() {
    let code = r#"
function fizzbuzz(n) {
    return match n {
        x where x % 15 == 0 => "fizzbuzz",
        x where x % 3 == 0 => "fizz",
        x where x % 5 == 0 => "buzz",
        _ => "number"
    }
}
fizzbuzz(15)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("fizzbuzz");
}

#[test]
fn pm_12_modulo_guard_fizz() {
    let code = r#"
function fizzbuzz(n) {
    return match n {
        x where x % 15 == 0 => "fizzbuzz",
        x where x % 3 == 0 => "fizz",
        x where x % 5 == 0 => "buzz",
        _ => "number"
    }
}
fizzbuzz(9)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("fizz");
}

#[test]
fn pm_12_modulo_guard_buzz() {
    let code = r#"
function fizzbuzz(n) {
    return match n {
        x where x % 15 == 0 => "fizzbuzz",
        x where x % 3 == 0 => "fizz",
        x where x % 5 == 0 => "buzz",
        _ => "number"
    }
}
fizzbuzz(10)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("buzz");
}

#[test]
fn pm_12_modulo_guard_number() {
    let code = r#"
function fizzbuzz(n) {
    return match n {
        x where x % 15 == 0 => "fizzbuzz",
        x where x % 3 == 0 => "fizz",
        x where x % 5 == 0 => "buzz",
        _ => "number"
    }
}
fizzbuzz(7)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("number");
}

#[test]
fn pm_12_guard_with_or() {
    let code = r#"
function classify(n) {
    return match n {
        x where x < -100 or x > 100 => "extreme",
        _ => "normal"
    }
}
classify(200)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("extreme");
}

#[test]
fn pm_12_guard_with_or_negative() {
    let code = r#"
function classify(n) {
    return match n {
        x where x < -100 or x > 100 => "extreme",
        _ => "normal"
    }
}
classify(-200)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("extreme");
}

#[test]
fn pm_12_guard_with_or_normal() {
    let code = r#"
function classify(n) {
    return match n {
        x where x < -100 or x > 100 => "extreme",
        _ => "normal"
    }
}
classify(50)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("normal");
}

// ============================================================================
// 13. Using matched variables inside the arm body
// ============================================================================

#[test]
fn pm_13_bound_variable_in_arm_body() {
    let code = r#"
function double_if_positive(n) {
    return match n {
        x where x > 0 => x * 2,
        _ => 0
    }
}
double_if_positive(5)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(10.0);
}

#[test]
fn pm_13_bound_variable_in_arm_body_wildcard() {
    let code = r#"
function double_if_positive(n) {
    return match n {
        x where x > 0 => x * 2,
        _ => 0
    }
}
double_if_positive(-3)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(0.0);
}

#[test]
fn pm_13_bound_variable_used_in_computation() {
    let code = r#"
function compute(n) {
    return match n {
        x where x > 0 => x * x + 1,
        x where x < 0 => x * x - 1,
        _ => 0
    }
}
compute(3)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(10.0);
}

#[test]
fn pm_13_bound_variable_negative_computation() {
    let code = r#"
function compute(n) {
    return match n {
        x where x > 0 => x * x + 1,
        x where x < 0 => x * x - 1,
        _ => 0
    }
}
compute(-3)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(8.0);
}

// ============================================================================
// 14. Match as last expression (implicit return)
// ============================================================================

#[test]
fn pm_14_match_implicit_return() {
    let code = r#"
function label(n) {
    match n {
        x where x > 0 => "positive",
        _ => "non-positive"
    }
}
label(5)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("positive");
}

#[test]
fn pm_14_match_implicit_return_wildcard() {
    let code = r#"
function label(n) {
    match n {
        x where x > 0 => "positive",
        _ => "non-positive"
    }
}
label(-1)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("non-positive");
}
