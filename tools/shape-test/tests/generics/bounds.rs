//! Trait-bounded generics: single bounds, multi-bounds, where clauses.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Single trait bound
// =========================================================================

#[test]
fn bounded_generic_single_trait_parses() {
    ShapeTest::new(
        r#"
        fn render<T: Display>(x: T) -> string {
            return "rendered"
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn bounded_generic_calls_trait_method() {
    // TDD: trait bound should allow calling the bounded method
    ShapeTest::new(
        r#"
        trait Describable {
            describe(self): string
        }

        type Item { name: string }

        impl Describable for Item {
            method describe() { self.name }
        }

        fn show<T: Describable>(x: T) -> string {
            return x.describe()
        }

        show(Item { name: "widget" })
    "#,
    )
    .expect_string("widget");
}

// =========================================================================
// Multi-bound generics
// =========================================================================

#[test]
fn multi_bound_generic_parses() {
    ShapeTest::new(
        r#"
        fn process<T: Serializable + Display>(x: T) {
            return x
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn multi_bound_generic_three_traits_parses() {
    ShapeTest::new(
        r#"
        fn transform<T: Display + Serializable + Comparable>(x: T) -> T {
            return x
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Where clauses
// =========================================================================

#[test]
fn where_clause_single_bound_parses() {
    ShapeTest::new(
        r#"
        fn sort<T>(items: Array<T>) -> Array<T>
            where T: Comparable
        {
            return items
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn where_clause_multiple_predicates_parses() {
    ShapeTest::new(
        r#"
        fn merge<T, U>(a: T, b: U) -> string
            where T: Serializable + Display, U: Comparable
        {
            return "merged"
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Bound with default type param
// =========================================================================

#[test]
fn bounded_generic_with_default_parses() {
    ShapeTest::new(
        r#"
        fn foo<T: Comparable = int>(x: T) -> T {
            return x
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn where_clause_with_function_body() {
    // TDD: where clause with actual logic in the function body
    ShapeTest::new(
        r#"
        fn identity<T>(x: T) -> T where T: Display {
            return x
        }
        identity(42)
    "#,
    )
    .expect_number(42.0);
}
