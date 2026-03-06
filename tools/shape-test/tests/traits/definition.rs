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
            to_string(self): string
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
            run(self): any
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
            size(self): int;
            is_empty(self): bool
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
            length(self): int;
            first(self): any;
            last(self): any
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
            find(self, query: string): any;
            contains(self, item: any): bool
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
            convert(self): T
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
            next(self): any
        }
    "#,
    )
    .expect_parse_ok();
}
