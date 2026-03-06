//! Type aliases: simple aliases, using aliases in signatures, alias chains.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Simple type aliases
// =========================================================================

#[test]
fn alias_number_type_parses() {
    ShapeTest::new(
        r#"
        type Percent = number
        let x: Percent = 0.15
        x
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn alias_int_type_parses() {
    ShapeTest::new(
        r#"
        type ID = int
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn alias_string_type_parses() {
    ShapeTest::new(
        r#"
        type Name = string
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Aliases in function signatures
// =========================================================================

#[test]
fn alias_in_return_type_parses() {
    ShapeTest::new(
        r#"
        type Score = number
        fn get_score() -> Score {
            return 100.0
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn alias_in_param_type_parses() {
    ShapeTest::new(
        r#"
        type ID = int
        fn lookup(id: ID) -> string {
            return "found"
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Alias chains
// =========================================================================

#[test]
fn alias_chain_parses() {
    ShapeTest::new(
        r#"
        type A = number
        type B = A
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Struct alias
// =========================================================================

#[test]
fn alias_to_struct_parses() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        type P = Point
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Array type alias
// =========================================================================

#[test]
fn alias_array_type_parses() {
    ShapeTest::new(
        r#"
        type IntList = Array<int>
    "#,
    )
    .expect_parse_ok();
}
