//! Union types: type alias unions, inline union in parameters.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Union type alias
// =========================================================================

#[test]
fn union_type_alias_parses() {
    ShapeTest::new(
        r#"
        type StringOrInt = string | int
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn union_type_three_variants_parses() {
    ShapeTest::new(
        r#"
        type Value = string | int | bool
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn union_type_with_named_types_parses() {
    // TDD: union of user-defined types
    ShapeTest::new(
        r#"
        type Success { value: string }
        type Failure { error: string }
        type Result = Success | Failure
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Union in parameter types
// =========================================================================

#[test]
fn union_in_function_param_parses() {
    // TDD: inline union type in function parameter
    ShapeTest::new(
        r#"
        fn accept(x: string | int) -> string {
            return "ok"
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn union_in_return_type_parses() {
    // TDD: inline union in return type
    ShapeTest::new(
        r#"
        fn maybe_int() -> int | string {
            return 42
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Union + intersection combined
// =========================================================================

#[test]
fn union_intersection_combined_parses() {
    ShapeTest::new(
        r#"
        type Mixed = A | B + C
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Union with runtime value
// =========================================================================

#[test]
fn union_type_alias_holds_int_value() {
    // TDD: storing an int in a union-typed variable
    ShapeTest::new(
        r#"
        type Flexible = string | int
        let x: Flexible = 42
        print(x)
    "#,
    )
    .expect_output("42");
}

#[test]
fn union_type_alias_holds_string_value() {
    // TDD: storing a string in a union-typed variable
    ShapeTest::new(
        r#"
        type Flexible = string | int
        let x: Flexible = "hello"
        print(x)
    "#,
    )
    .expect_output("hello");
}
