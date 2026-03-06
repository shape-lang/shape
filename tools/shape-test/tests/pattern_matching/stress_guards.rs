//! Stress tests for guard clauses (where) in match patterns.
//!
//! Migrated from shape-vm stress_18_patterns.rs — Section 6, plus guard-related
//! tests from Sections 9, 13, 16, and 19.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 6: Guard Clauses (tests 43-52)
// =============================================================================

/// Basic guard with greater-than.
#[test]
fn t43_guard_basic() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 4
            return match x {
                n where n > 3 => n,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(4.0);
}

/// Guard fails, goes to next arm.
#[test]
fn t44_guard_fails_goes_to_next_arm() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 2
            return match x {
                n where n > 3 => 100,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(0.0);
}

/// Guard with equality check.
#[test]
fn t45_guard_with_equality() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 10
            return match x {
                n where n == 10 => 1,
                n where n == 20 => 2,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Second guard arm matches.
#[test]
fn t46_guard_second_arm_matches() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 20
            return match x {
                n where n == 10 => 1,
                n where n == 20 => 2,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Complex guard condition with and.
#[test]
fn t47_guard_complex_condition() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 15
            return match x {
                n where n > 0 and n < 10 => 1,
                n where n >= 10 and n < 20 => 2,
                n where n >= 20 => 3,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Guard with negative check.
#[test]
fn t48_guard_negative_check() {
    ShapeTest::new(
        r#"
        function test() {
            let x = -5
            return match x {
                n where n > 0 => 1,
                n where n < 0 => -1,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(-1.0);
}

/// Guard with literal arm mix.
#[test]
fn t49_guard_with_literal_arm_mix() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 0
            return match x {
                0 => 100,
                n where n > 0 => 1,
                _ => -1
            }
        }
        test()
    "#,
    )
    .expect_number(100.0);
}

/// All guards fail, wildcard catches.
#[test]
fn t50_guard_all_guards_fail() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 0
            return match x {
                n where n > 10 => 1,
                n where n < -10 => 2,
                _ => 99
            }
        }
        test()
    "#,
    )
    .expect_number(99.0);
}

/// Guard with modulo check.
#[test]
fn t51_guard_modulo_check() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 12
            return match x {
                n where n % 3 == 0 => 1,
                n where n % 2 == 0 => 2,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Guard uses outer variable.
#[test]
fn t52_guard_uses_outer_variable() {
    ShapeTest::new(
        r#"
        function test() {
            let threshold = 10
            let x = 15
            return match x {
                n where n > threshold => 1,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

// =============================================================================
// Guard + Constructor patterns (from Section 9)
// =============================================================================

/// Option with guard on inner value.
#[test]
fn t80_match_option_with_guard() {
    ShapeTest::new(
        r#"
        function test() {
            let x = Some(5)
            return match x {
                Some(v) where v > 10 => 1,
                Some(v) => 2,
                None => 0
            }
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

// =============================================================================
// Guard + method calls (from Section 13)
// =============================================================================

/// Guard with string length method call.
#[test]
fn t113_guard_with_string_length() {
    ShapeTest::new(
        r#"
        function test() {
            let s = "hello"
            return match s {
                x where x.length() > 3 => 1,
                _ => 0
            }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

// =============================================================================
// Literal + Guard + Wildcard combined (from Section 16)
// =============================================================================

/// Classify negative with literal+guard+wildcard.
#[test]
fn t126_match_literal_then_guard_then_wildcard() {
    ShapeTest::new(
        r#"
        function classify(n) {
            return match n {
                0 => "zero",
                n where n > 0 => "positive",
                _ => "negative"
            }
        }
        function test() {
            return classify(-5)
        }
        test()
    "#,
    )
    .expect_string("negative");
}

/// Classify positive with literal+guard+wildcard.
#[test]
fn t127_classify_positive() {
    ShapeTest::new(
        r#"
        function classify(n) {
            return match n {
                0 => "zero",
                n where n > 0 => "positive",
                _ => "negative"
            }
        }
        function test() {
            return classify(10)
        }
        test()
    "#,
    )
    .expect_string("positive");
}

/// Classify zero with literal+guard+wildcard.
#[test]
fn t128_classify_zero() {
    ShapeTest::new(
        r#"
        function classify(n) {
            return match n {
                0 => "zero",
                n where n > 0 => "positive",
                _ => "negative"
            }
        }
        function test() {
            return classify(0)
        }
        test()
    "#,
    )
    .expect_string("zero");
}

// =============================================================================
// Guard edge cases (from Section 19)
// =============================================================================

/// Multiple guards with same identifier; first matching wins.
#[test]
fn t142_match_with_multiple_guards_same_identifier() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 50
            return match x {
                n where n < 10 => 1,
                n where n < 100 => 2,
                n where n < 1000 => 3,
                _ => 4
            }
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// All guards false, falls through to wildcard.
#[test]
fn t143_match_guard_false_falls_through() {
    ShapeTest::new(
        r#"
        function test() {
            let x = 5
            return match x {
                n where n > 100 => 1,
                n where n > 50 => 2,
                _ => 3
            }
        }
        test()
    "#,
    )
    .expect_number(3.0);
}
