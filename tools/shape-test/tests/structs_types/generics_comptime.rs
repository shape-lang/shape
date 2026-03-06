//! Generic type parameters, trait bounds, where clauses,
//! and comptime field tests.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 6. Generics (15 tests)
// =========================================================================

#[test]
fn generic_identity_function_int() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { x }
        id(42)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn generic_identity_function_string() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { x }
        id("hello")
    "#,
    )
    .expect_string("hello");
}

#[test]
fn generic_identity_function_bool() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { x }
        id(true)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn generic_struct_definition_and_use() {
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
fn generic_struct_with_string() {
    ShapeTest::new(
        r#"
        type Box<T> { value: T }
        let b = Box { value: "hello" }
        b.value
    "#,
    )
    .expect_string("hello");
}

#[test]
fn generic_function_two_type_params_parses() {
    ShapeTest::new(
        r#"
        fn pair<A, B>(a: A, b: B) -> A { a }
        pair(1, "hello")
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn generic_struct_two_params() {
    ShapeTest::new(
        r#"
        type Pair<A, B> { first: A, second: B }
        let p = Pair { first: 1, second: "hello" }
        p.first
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn generic_struct_two_params_second_field() {
    ShapeTest::new(
        r#"
        type Pair<A, B> { first: A, second: B }
        let p = Pair { first: 1, second: "hello" }
        p.second
    "#,
    )
    .expect_string("hello");
}

#[test]
fn generic_function_with_trait_bound_parses() {
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
fn generic_function_with_multiple_bounds_parses() {
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
fn generic_type_param_default_parses() {
    ShapeTest::new(
        r#"
        fn foo<T = int>(x: T) {
            return x
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn generic_type_param_bound_with_default_parses() {
    ShapeTest::new(
        r#"
        fn foo<T: Numeric = int>(x: T) {
            return x
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn generic_where_clause_parses() {
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
fn generic_where_clause_multiple_predicates_parses() {
    ShapeTest::new(
        r#"
        fn process<T, U>(a: T, b: U) -> string
            where T: Serializable + Display, U: Comparable
        {
            return ""
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn generic_struct_with_array_field() {
    ShapeTest::new(
        r#"
        type Series<V, K> { index: Array<K>, data: Array<V> }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// 7. Comptime fields on types (5 tests)
// =========================================================================

#[test]
fn comptime_field_static_access() {
    ShapeTest::new(
        r#"
        type Currency {
            comptime symbol: string = "$",
            amount: number
        }
        Currency.symbol
    "#,
    )
    .expect_string("$");
}

#[test]
fn comptime_field_numeric_default() {
    ShapeTest::new(
        r#"
        type Percent {
            comptime decimals: number = 2,
            value: number
        }
        Percent.decimals
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn comptime_field_with_runtime_field() {
    ShapeTest::new(
        r#"
        type Currency {
            comptime symbol: string = "$",
            amount: number
        }
        let c = Currency { amount: 100 }
        c.amount
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn comptime_field_multiple_parses() {
    ShapeTest::new(
        r#"
        type Currency {
            comptime symbol: string = "$",
            comptime decimals: number = 2,
            amount: number
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn comptime_field_no_default_parses() {
    ShapeTest::new(
        r#"
        type Measurement {
            comptime unit: string,
            value: number
        }
    "#,
    )
    .expect_parse_ok();
}
