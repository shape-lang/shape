//! Multi-parameter generics: multiple type params, HashMap<K,V>, generic structs with two params.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Two type parameters in functions
// =========================================================================

#[test]
fn two_type_params_returns_first() {
    ShapeTest::new(
        r#"
        fn first<A, B>(a: A, b: B) -> A { a }
        first(42, "hello")
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn two_type_params_returns_second() {
    ShapeTest::new(
        r#"
        fn second<A, B>(a: A, b: B) -> B { b }
        second(42, "hello")
    "#,
    )
    .expect_string("hello");
}

// =========================================================================
// Multi-param generic structs
// =========================================================================

#[test]
fn generic_pair_struct_first_field() {
    ShapeTest::new(
        r#"
        type Pair<A, B> { first: A, second: B }
        let p = Pair { first: 10, second: "ten" }
        p.first
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn generic_pair_struct_second_field() {
    ShapeTest::new(
        r#"
        type Pair<A, B> { first: A, second: B }
        let p = Pair { first: 10, second: "ten" }
        p.second
    "#,
    )
    .expect_string("ten");
}

#[test]
fn generic_struct_with_three_params_parses() {
    ShapeTest::new(
        r#"
        type Triple<A, B, C> { x: A, y: B, z: C }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// HashMap<K, V> style multi-param generics
// =========================================================================

#[test]
fn hashmap_generic_type_parses() {
    // TDD: HashMap<K, V> multi-param generic in type position
    ShapeTest::new(
        r#"
        fn get_map() -> HashMap<string, int> {
            return HashMap {}
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn generic_struct_array_field_parses() {
    ShapeTest::new(
        r#"
        type Series<V, K> { index: Array<K>, data: Array<V> }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Default type parameters
// =========================================================================

#[test]
fn default_type_param_parses() {
    // TDD: default type parameters may not be fully supported at runtime
    ShapeTest::new(
        r#"
        fn foo<T = int>(x: T) -> T {
            return x
        }
    "#,
    )
    .expect_parse_ok();
}
