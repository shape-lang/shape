//! Stress tests for generic functions, generic structs, generic with struct,
//! generic with array return, generic identity with null, and complex
//! generic interactions.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 8. GENERIC FUNCTIONS
// =========================================================================

/// Verifies generic identity int.
#[test]
fn generic_identity_int() {
    ShapeTest::new(
        r#"
        fn identity<T>(x: T) -> T { return x }
        fn test() { return identity(42) }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies generic identity string.
#[test]
fn generic_identity_string() {
    ShapeTest::new(
        r#"
        fn identity<T>(x: T) -> T { return x }
        fn test() { return identity("hello") }
        test()
    "#,
    )
    .expect_string("hello");
}

/// Verifies generic identity bool.
#[test]
fn generic_identity_bool() {
    ShapeTest::new(
        r#"
        fn identity<T>(x: T) -> T { return x }
        fn test() { return identity(true) }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies generic identity number.
#[test]
fn generic_identity_number() {
    ShapeTest::new(
        r#"
        fn identity<T>(x: T) -> T { return x }
        fn test() { return identity(2.718) }
        test()
    "#,
    )
    .expect_number(2.718);
}

/// Verifies generic two params.
#[test]
fn generic_two_params() {
    ShapeTest::new(
        r#"
        fn first_of<A, B>(a: A, b: B) -> A { return a }
        fn test() { return first_of(10, "ignored") }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Verifies generic two params return second.
#[test]
fn generic_two_params_return_second() {
    ShapeTest::new(
        r#"
        fn second_of<A, B>(a: A, b: B) -> B { return b }
        fn test() { return second_of(10, "picked") }
        test()
    "#,
    )
    .expect_string("picked");
}

/// Verifies generic function called multiple times.
#[test]
fn generic_function_called_multiple_times() {
    ShapeTest::new(
        r#"
        fn identity<T>(x: T) -> T { return x }
        fn test() {
            let a = identity(1)
            let b = identity("two")
            let c = identity(true)
            return a
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies generic runtime type via type method.
#[test]
fn generic_runtime_type_via_type_method() {
    ShapeTest::new(
        r#"
        fn inner<T>(x: T) {
            return x.type().to_string()
        }
        fn test() { return inner(2.1) }
        test()
    "#,
    )
    .expect_string("number");
}

/// Verifies generic runtime type via type method int.
#[test]
fn generic_runtime_type_via_type_method_int() {
    ShapeTest::new(
        r#"
        fn inner<T>(x: T) {
            return x.type().to_string()
        }
        fn test() { return inner(42) }
        test()
    "#,
    )
    .expect_string("int");
}

// =========================================================================
// 12. GENERIC STRUCT TYPES
// =========================================================================

/// Verifies generic struct default type param.
#[test]
fn generic_struct_default_type_param() {
    ShapeTest::new(
        r#"
        type Wrapper<T = int> { value: T }
        fn test() {
            let w = Wrapper { value: 42 }
            return w.value
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies generic struct inferred type arg.
#[test]
fn generic_struct_inferred_type_arg() {
    ShapeTest::new(
        r#"
        type Wrapper<T = int> { value: T }
        fn test() {
            let w = Wrapper { value: 3.14 }
            return w.value
        }
        test()
    "#,
    )
    .expect_number(3.14);
}

/// Verifies generic struct type name with default.
#[test]
fn generic_struct_type_name_with_default() {
    ShapeTest::new(
        r#"
        type MyType<T = int> { x: T }
        fn test() {
            let a = MyType { x: 1 }
            return a.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("MyType");
}

/// Verifies generic struct type name with non default.
#[test]
fn generic_struct_type_name_with_non_default() {
    ShapeTest::new(
        r#"
        type MyType<T = int> { x: T }
        fn test() {
            let a = MyType { x: 1.0 }
            return a.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("MyType<number>");
}

/// Verifies generic struct two fields.
#[test]
fn generic_struct_two_fields() {
    ShapeTest::new(
        r#"
        type Pair<A = int, B = int> { first: A, second: B }
        fn test() {
            let p = Pair { first: 1, second: 2 }
            return p.first + p.second
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

// =========================================================================
// 18. MULTIPLE GENERIC PARAMS
// =========================================================================

/// Verifies multi generic function two types.
#[test]
fn multi_generic_function_two_types() {
    ShapeTest::new(
        r#"
        fn pick_first<A, B>(a: A, b: B) -> A { return a }
        fn test() { return pick_first("hello", 42) }
        test()
    "#,
    )
    .expect_string("hello");
}

/// Verifies multi generic three params.
#[test]
fn multi_generic_three_params() {
    ShapeTest::new(
        r#"
        fn third<A, B, C>(a: A, b: B, c: C) -> C { return c }
        fn test() { return third(1, "two", true) }
        test()
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// 30. GENERIC FUNCTION WITH STRUCT
// =========================================================================

/// Verifies generic fn accepts struct.
#[test]
fn generic_fn_accepts_struct() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        fn wrap<T>(val: T) -> T { return val }
        fn test() {
            let p = Point { x: 3, y: 4 }
            let w = wrap(p)
            return w.x
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

// =========================================================================
// 37. GENERIC WITH DIFFERENT CALL SITES
// =========================================================================

/// Verifies generic fn different instantiations.
#[test]
fn generic_fn_different_instantiations() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { return x }
        fn test() {
            let a = id(100)
            let b = id("abc")
            return a
        }
        test()
    "#,
    )
    .expect_number(100.0);
}

/// Verifies generic fn chain calls.
#[test]
fn generic_fn_chain_calls() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { return x }
        fn test() {
            return id(id(id(42)))
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies generic fn with array return.
#[test]
fn generic_fn_with_array_return() {
    ShapeTest::new(
        r#"
        fn wrap_in_array<T>(x: T) {
            return [x]
        }
        fn test() {
            let arr = wrap_in_array(42)
            return arr[0]
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies generic identity with null.
#[test]
fn generic_identity_with_null() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { return x }
        fn test() {
            let x = id(None)
            return x == None
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies struct passed to generic fn.
#[test]
fn struct_passed_to_generic_fn() {
    ShapeTest::new(
        r#"
        type Data { value: int }
        fn extract<T>(x: T) -> T { return x }
        fn test() {
            let d = Data { value: 99 }
            let e = extract(d)
            return e.value
        }
        test()
    "#,
    )
    .expect_number(99.0);
}
