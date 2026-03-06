//! Stress tests for variable reassignment, different-name bindings,
//! for-loop variable scoping, and mutable variable updates.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 4. Reassignment (var)
// =========================================================================

/// Verifies mutable variable reassignment in same scope.
#[test]
fn test_shadow_same_scope() {
    ShapeTest::new("var x = 1\nx = 2\nx").expect_number(2.0);
}

/// Verifies reassignment uses previous value.
#[test]
fn test_shadow_uses_previous_value() {
    ShapeTest::new(
        "fn test() -> int {
            var x = 1
            x = x + 1
            return x
        }\ntest()",
    )
    .expect_number(2.0);
}

/// Verifies chain of reassignments.
#[test]
fn test_shadow_chain() {
    ShapeTest::new(
        "fn test() -> int {
            var x = 1
            x = x + 1
            x = x + 1
            x = x + 1
            return x
        }\ntest()",
    )
    .expect_number(4.0);
}

/// Verifies binding different types to different variable names (int then string).
#[test]
fn test_shadow_different_type_int_to_string() {
    ShapeTest::new(
        "fn test() -> string {
            let x = 42
            let y = \"hello\"
            return y
        }\ntest()",
    )
    .expect_string("hello");
}

/// Verifies binding different types to different variable names (string then int).
#[test]
fn test_shadow_different_type_string_to_int() {
    ShapeTest::new(
        "fn test() -> int {
            let x = \"hello\"
            let y = 99
            return y
        }\ntest()",
    )
    .expect_number(99.0);
}

/// Verifies binding different types to different variable names (bool then int).
#[test]
fn test_shadow_different_type_bool_to_int() {
    ShapeTest::new(
        "fn test() -> int {
            let x = true
            let y = 7
            return y
        }\ntest()",
    )
    .expect_number(7.0);
}

// =========================================================================
// For-loop variable scoping
// =========================================================================

/// Verifies for-loop variable does not leak to outer scope.
#[test]
fn test_shadow_in_for_loop() {
    ShapeTest::new(
        "fn test() -> int {
            let i = 99
            var sum = 0
            for i in 1..4 {
                sum = sum + i
            }
            return i
        }\ntest()",
    )
    .expect_number(99.0);
}

// =========================================================================
// 28. Mutable variable with updates
// =========================================================================

/// Verifies mutable variable with arithmetic updates.
#[test]
fn test_shadow_let_with_var() {
    ShapeTest::new(
        "fn test() -> int {
            var x = 1
            x = x + 10
            x = x + 5
            return x
        }\ntest()",
    )
    .expect_number(16.0);
}
