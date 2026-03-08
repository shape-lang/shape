//! Stress tests for trait declarations: single/multiple required methods, empty traits,
//! traits with params, various return types, type params, semicolons/commas separators,
//! extends rejection, and trait definition only (no impl).

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. BASIC TRAIT DECLARATION
// =========================================================================

/// Verifies trait single required method.
#[test]
fn trait_single_required_method() {
    ShapeTest::new(
        r#"
        trait Printable {
            display(self): string
        }
        42
    "#,
    )
    .expect_number(42.0);
}

/// Verifies trait multiple required methods.
#[test]
fn trait_multiple_required_methods() {
    ShapeTest::new(
        r#"
        trait Shape {
            area(self): number,
            perimeter(self): number,
            name(self): string
        }
        1
    "#,
    )
    .expect_number(1.0);
}

/// Verifies trait no methods empty body.
#[test]
fn trait_no_methods_empty_body() {
    ShapeTest::new(
        r#"
        trait Marker {}
        100
    "#,
    )
    .expect_number(100.0);
}

/// Verifies trait method with params.
#[test]
fn trait_method_with_params() {
    ShapeTest::new(
        r#"
        trait Transformer {
            transform(self, x: number): number
        }
        1
    "#,
    )
    .expect_number(1.0);
}

/// Verifies trait method returns bool.
#[test]
fn trait_method_returns_bool() {
    ShapeTest::new(
        r#"
        trait Validator {
            is_valid(self): bool
        }
        true
    "#,
    )
    .expect_bool(true);
}

/// Verifies trait method returns int.
#[test]
fn trait_method_returns_int() {
    ShapeTest::new(
        r#"
        trait Countable {
            count(self): int
        }
        7
    "#,
    )
    .expect_number(7.0);
}

// =========================================================================
// 25. TRAIT WITH INTERFACE-STYLE SEMICOLONS
// =========================================================================

/// Verifies trait members separated by semicolons.
#[test]
fn trait_members_separated_by_semicolons() {
    ShapeTest::new(
        r#"
        trait Shape {
            area(self): number;
            name(self): string;
        }
        1
    "#,
    )
    .expect_number(1.0);
}

/// Verifies trait members separated by commas.
#[test]
fn trait_members_separated_by_commas() {
    ShapeTest::new(
        r#"
        trait Shape {
            area(self): number,
            name(self): string
        }
        1
    "#,
    )
    .expect_number(1.0);
}

// =========================================================================
// 26. EXTENDS SYNTAX REJECTED
// =========================================================================

/// Verifies trait extends is rejected.
#[test]
fn trait_extends_is_rejected() {
    ShapeTest::new(
        r#"
        trait Base { base_method(self): string }
        trait Derived extends Base { derived_method(self): string }
        1
    "#,
    )
    .expect_run_err();
}

// =========================================================================
// 12. TRAIT WITH TYPE PARAMETERS
// =========================================================================

/// Verifies trait with type param.
/// TDD: impl for primitive types with generic trait params doesn't resolve methods at runtime.
#[test]
fn trait_with_type_param() {
    ShapeTest::new(
        r#"
        trait Boxed<T> {
            contents(self): T
        }
        impl Boxed<number> for number {
            method contents() { self }
        }
        let b = 100.0
        b.contents()
    "#,
    )
    .expect_run_err();
}

/// Verifies trait with type param compiles.
#[test]
fn trait_with_type_param_compiles() {
    ShapeTest::new(
        r#"
        trait Queryable<T> {
            filter(predicate): any,
            execute(): any
        }
        1
    "#,
    )
    .expect_number(1.0);
}

// =========================================================================
// 38. TRAIT DEFINITION -- VARIOUS SIGNATURES
// =========================================================================

/// Verifies trait no self param.
#[test]
fn trait_no_self_param() {
    ShapeTest::new(
        r#"
        trait Factory {
            create(): any
        }
        1
    "#,
    )
    .expect_number(1.0);
}

/// Verifies trait method no return type.
#[test]
fn trait_method_no_return_type() {
    ShapeTest::new(
        r#"
        trait Processor {
            process(self, data: string): any
        }
        1
    "#,
    )
    .expect_number(1.0);
}

// =========================================================================
// 69. TRAIT DECLARATION ONLY (NO IMPL, NO USE)
// =========================================================================

/// Verifies trait declaration only.
#[test]
fn trait_declaration_only() {
    ShapeTest::new(
        r#"
        trait Serializable {
            serialize(self): string,
            deserialize(data: string): any
        }
        "ok"
    "#,
    )
    .expect_string("ok");
}

// =========================================================================
// 70. MULTIPLE TRAIT DECLARATIONS
// =========================================================================

/// Verifies multiple trait declarations.
#[test]
fn multiple_trait_declarations() {
    ShapeTest::new(
        r#"
        trait A { a_method(self): int }
        trait B { b_method(self): int }
        trait C { c_method(self): string }
        trait D { d_method(self): bool }
        "all declared"
    "#,
    )
    .expect_string("all declared");
}

// =========================================================================
// 87. TRAIT — WHERE CLAUSE (COMPILATION)
// =========================================================================

/// Where clauses should parse and compile.
#[test]
fn impl_with_where_clause_compiles() {
    ShapeTest::new(
        r#"
        trait Printable { show(self): string }
        type Container { val: int }
        impl Printable for Container where Container: Printable {
            method show() { "container" }
        }
        let c = Container { val: 1 }
        c.show()
    "#,
    )
    .expect_string("container");
}
