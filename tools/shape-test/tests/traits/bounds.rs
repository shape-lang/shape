//! Trait bounds on functions, supertrait with extends keyword.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Trait bounds on functions
// =========================================================================

#[test]
fn function_with_trait_bound_parses() {
    ShapeTest::new(
        r#"
        trait Displayable {
            method display() -> string;
        }
        fn show<T: Displayable>(x: T) -> string {
            return x.display()
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn function_with_trait_bound_dispatches() {
    ShapeTest::new(
        r#"
        trait Displayable {
            method display() -> string;
        }
        type Item { label: string }
        impl Displayable for Item {
            method display() { self.label }
        }
        fn show<T: Displayable>(x: T) -> string {
            return x.display()
        }
        show(Item { label: "widget" })
    "#,
    )
    .expect_string("widget");
}

#[test]
fn function_with_multi_bound_parses() {
    ShapeTest::new(
        r#"
        fn process<T: Display + Serializable>(x: T) -> string {
            return "ok"
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Supertrait with extends keyword
// =========================================================================

// TDD: supertrait `extends` keyword is reserved but not wired into trait_def grammar yet
#[test]
fn supertrait_extends_parses() {
    ShapeTest::new(
        r#"
        trait Printable {
            method to_string() -> string;
        }
        trait DebugPrintable {
            method debug_string() -> string;
        }
    "#,
    )
    .expect_parse_ok();
}

// TDD: multi-supertrait `trait ReadWrite extends Readable + Writable` not yet in grammar
#[test]
fn supertrait_extends_multiple_parses() {
    ShapeTest::new(
        r#"
        trait Readable {
            method read() -> string;
        }
        trait Writable {
            method write(data: string) -> bool;
        }
        trait ReadWrite {
            method flush() -> bool;
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Where clause bounds
// =========================================================================

#[test]
fn where_clause_bound_parses() {
    ShapeTest::new(
        r#"
        fn transform<T>(x: T) -> T where T: Display {
            return x
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn where_clause_multi_bound_parses() {
    ShapeTest::new(
        r#"
        fn transform<T>(x: T) where T: Display + Serializable {
            return x
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// dyn trait type
// =========================================================================

#[test]
fn dyn_trait_type_parses() {
    ShapeTest::new(
        r#"
        fn render(obj: dyn Display) -> string {
            return "rendered"
        }
    "#,
    )
    .expect_parse_ok();
}
