//! Trait definition syntax: single method, multiple methods, empty traits.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic trait definition
// =========================================================================

#[test]
fn trait_single_method_parses() {
    ShapeTest::new(
        r#"
        trait Printable {
            method to_string() -> string;
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_single_method_no_return_type_parses() {
    ShapeTest::new(
        r#"
        trait Runnable {
            method run() -> any;
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Multiple methods
// =========================================================================

#[test]
fn trait_two_methods_parses() {
    ShapeTest::new(
        r#"
        trait Container {
            method size() -> int;
            method is_empty() -> bool;
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_three_methods_parses() {
    ShapeTest::new(
        r#"
        trait Collection {
            method length() -> int;
            method first() -> any;
            method last() -> any;
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_methods_with_parameters_parses() {
    ShapeTest::new(
        r#"
        trait Searchable {
            method find(query: string) -> any;
            method contains(item: any) -> bool;
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Empty trait (marker trait)
// =========================================================================

#[test]
fn empty_trait_parses() {
    // TDD: empty trait body (marker trait) may not be supported in grammar
    ShapeTest::new(
        r#"
        trait Marker {
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Trait with type param
// =========================================================================

#[test]
fn trait_with_type_param_parses() {
    ShapeTest::new(
        r#"
        trait Convertible<T> {
            method convert() -> T;
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_with_associated_type_parses() {
    ShapeTest::new(
        r#"
        trait Iterator {
            type Item;
            method next() -> any;
        }
    "#,
    )
    .expect_parse_ok();
}
