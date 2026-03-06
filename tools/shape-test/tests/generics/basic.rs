//! Basic generics tests: generic functions, generic structs, type inference at call sites.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Generic identity function
// =========================================================================

#[test]
fn generic_identity_with_int() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { x }
        id(42)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn generic_identity_with_string() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { x }
        id("hello")
    "#,
    )
    .expect_string("hello");
}

#[test]
fn generic_identity_with_bool() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { x }
        id(true)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn generic_identity_preserves_negative_number() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { x }
        id(-3.14)
    "#,
    )
    .expect_number(-3.14);
}

// =========================================================================
// Generic struct
// =========================================================================

#[test]
fn generic_struct_with_int_value() {
    ShapeTest::new(
        r#"
        type Box<T> { value: T }
        let b = Box { value: 42 }
        b.value
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn generic_struct_with_string_value() {
    ShapeTest::new(
        r#"
        type Box<T> { value: T }
        let b = Box { value: "world" }
        b.value
    "#,
    )
    .expect_string("world");
}

#[test]
fn generic_struct_with_bool_value() {
    ShapeTest::new(
        r#"
        type Box<T> { value: T }
        let b = Box { value: false }
        b.value
    "#,
    )
    .expect_bool(false);
}

// =========================================================================
// Generic call-site type inference
// =========================================================================

#[test]
fn generic_function_infers_type_from_argument() {
    // TDD: type inference should pick up the argument type without explicit annotation
    ShapeTest::new(
        r#"
        fn wrap<T>(x: T) -> T {
            return x
        }
        let result = wrap(100)
        print(result)
    "#,
    )
    .expect_output("100");
}

#[test]
fn generic_function_used_with_different_types() {
    // TDD: same generic function called with int then string
    ShapeTest::new(
        r#"
        fn echo<T>(x: T) -> T { x }
        let a = echo(10)
        let b = echo("hi")
        print(a)
        print(b)
    "#,
    )
    .expect_output("10\nhi");
}
