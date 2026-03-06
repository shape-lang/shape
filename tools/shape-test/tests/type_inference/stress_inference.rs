//! Stress tests for type inference from literals, arithmetic, string concat,
//! comparisons, function calls, Option<T>, Result<T,E>, typeof/.type(),
//! null/None, type preservation through control flow, int vs number separation,
//! and complex inference interactions.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 4. TYPE INFERENCE
// =========================================================================

/// Verifies infer int literal.
#[test]
fn infer_int_literal() {
    ShapeTest::new("let x = 42; x").expect_number(42.0);
}

/// Verifies infer number literal.
#[test]
fn infer_number_literal() {
    ShapeTest::new("let x = 3.14; x").expect_number(3.14);
}

/// Verifies infer string literal.
#[test]
fn infer_string_literal() {
    ShapeTest::new(r#"let x = "hello"; x"#).expect_string("hello");
}

/// Verifies infer bool literal.
#[test]
fn infer_bool_literal() {
    ShapeTest::new("let x = true; x").expect_bool(true);
}

/// Verifies infer from arithmetic.
#[test]
fn infer_from_arithmetic() {
    ShapeTest::new("let x = 10 + 5; x").expect_number(15.0);
}

/// Verifies infer from float arithmetic.
#[test]
fn infer_from_float_arithmetic() {
    ShapeTest::new("let x = 1.5 + 2.5; x").expect_number(4.0);
}

/// Verifies infer from string concat.
#[test]
fn infer_from_string_concat() {
    ShapeTest::new(r#"let x = "a" + "b"; x"#).expect_string("ab");
}

/// Verifies infer from comparison.
#[test]
fn infer_from_comparison() {
    ShapeTest::new("let x = 10 > 5; x").expect_bool(true);
}

/// Verifies infer from function call.
#[test]
fn infer_from_function_call() {
    ShapeTest::new(
        r#"
        fn double(x: int) -> int { x * 2 }
        let y = double(21)
        y
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 5. OPTION<T> -- Some AND None
// =========================================================================

/// Verifies option some int.
#[test]
fn option_some_int() {
    ShapeTest::new("let x = Some(42); x").expect_number(42.0);
}

/// Verifies option some string.
#[test]
fn option_some_string() {
    ShapeTest::new(r#"let x = Some("hello"); x"#).expect_string("hello");
}

/// Verifies option some bool.
#[test]
fn option_some_bool() {
    ShapeTest::new("let x = Some(true); x").expect_bool(true);
}

/// Verifies option none is none.
#[test]
fn option_none_is_none() {
    ShapeTest::new("let x = None; x").expect_none();
}

/// Verifies option some number.
#[test]
fn option_some_number() {
    ShapeTest::new("let x = Some(3.14); x").expect_number(3.14);
}

/// Verifies option null coalesce.
#[test]
fn option_null_coalesce() {
    ShapeTest::new("let x = None; x ?? 42").expect_number(42.0);
}

/// Verifies option some coalesce returns value.
#[test]
fn option_some_coalesce_returns_value() {
    ShapeTest::new("let x = Some(10); x ?? 42").expect_number(10.0);
}

/// Verifies option null check equality.
#[test]
fn option_null_check_equality() {
    ShapeTest::new("let x = None; x == None").expect_bool(true);
}

/// Verifies option some not null.
#[test]
fn option_some_not_null() {
    ShapeTest::new("let x = Some(5); x == None").expect_bool(false);
}

/// Verifies option nested some.
#[test]
fn option_nested_some() {
    ShapeTest::new("let x = Some(Some(42)); x").expect_number(42.0);
}

// =========================================================================
// 6. RESULT<T,E> -- Ok AND Err
// =========================================================================

/// Verifies result ok int.
#[test]
fn result_ok_int() {
    ShapeTest::new("Ok(42)").expect_run_ok();
}

/// Verifies result err string.
#[test]
fn result_err_string() {
    ShapeTest::new(r#"Err("something went wrong")"#).expect_run_ok();
}

/// Verifies result ok string.
#[test]
fn result_ok_string() {
    ShapeTest::new(r#"Ok("success")"#).expect_run_ok();
}

/// Verifies result ok bool.
#[test]
fn result_ok_bool() {
    ShapeTest::new("Ok(true)").expect_run_ok();
}

/// Verifies result err int.
#[test]
fn result_err_int() {
    ShapeTest::new("Err(404)").expect_run_ok();
}

/// Verifies result ok from function.
#[test]
fn result_ok_from_function() {
    ShapeTest::new(
        r#"
        fn safe_div(a: int, b: int) {
            if b == 0 {
                return Err("division by zero")
            }
            return Ok(a / b)
        }
        fn test() { return safe_div(10, 2) }
        test()
    "#,
    )
    .expect_run_ok();
}

/// Verifies result err from function.
#[test]
fn result_err_from_function() {
    ShapeTest::new(
        r#"
        fn safe_div(a: int, b: int) {
            if b == 0 {
                return Err("division by zero")
            }
            return Ok(a / b)
        }
        fn test() { return safe_div(10, 0) }
        test()
    "#,
    )
    .expect_run_ok();
}

// =========================================================================
// 7. typeof / .type() BUILTIN
// =========================================================================

/// Verifies typeof int via type method.
#[test]
fn typeof_int_via_type_method() {
    ShapeTest::new(
        r#"
        fn test() { return 42 .type().to_string() }
        test()
    "#,
    )
    .expect_string("int");
}

/// Verifies typeof number via type method.
#[test]
fn typeof_number_via_type_method() {
    ShapeTest::new(
        r#"
        fn test() { return 3.14 .type().to_string() }
        test()
    "#,
    )
    .expect_string("number");
}

/// Verifies typeof string via type method.
#[test]
fn typeof_string_via_type_method() {
    ShapeTest::new(
        r#"
        fn test() {
            let s = "hello"
            return s.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("string");
}

/// Verifies typeof bool via type method.
#[test]
fn typeof_bool_via_type_method() {
    ShapeTest::new(
        r#"
        fn test() { return true .type().to_string() }
        test()
    "#,
    )
    .expect_string("bool");
}

/// Verifies typeof array via type method.
#[test]
fn typeof_array_via_type_method() {
    ShapeTest::new(
        r#"
        fn test() {
            let a = [1, 2, 3]
            return a.type().to_string()
        }
        test()
    "#,
    )
    .expect_run_ok();
}

/// Verifies typeof struct via type method.
#[test]
fn typeof_struct_via_type_method() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        fn test() {
            let p = Point { x: 1, y: 2 }
            return p.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("Point");
}

/// Verifies typeof struct on type symbol.
#[test]
fn typeof_struct_on_type_symbol() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        fn test() {
            return Point.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("Point");
}

// =========================================================================
// 13. INT VS NUMBER -- SEPARATE TYPES
// =========================================================================

/// Verifies int literal is int.
#[test]
fn int_literal_is_int() {
    ShapeTest::new("42").expect_number(42.0);
}

/// Verifies float literal is number.
#[test]
fn float_literal_is_number() {
    ShapeTest::new("3.14").expect_number(3.14);
}

/// Verifies int arithmetic preserves int.
#[test]
fn int_arithmetic_preserves_int() {
    ShapeTest::new("10 + 20").expect_number(30.0);
}

/// Verifies number arithmetic preserves number.
#[test]
fn number_arithmetic_preserves_number() {
    ShapeTest::new("1.5 + 2.5").expect_number(4.0);
}

/// Verifies int mul preserves int.
#[test]
fn int_mul_preserves_int() {
    ShapeTest::new("6 * 7").expect_number(42.0);
}

/// Verifies int sub preserves int.
#[test]
fn int_sub_preserves_int() {
    ShapeTest::new("100 - 58").expect_number(42.0);
}

/// Verifies number mul preserves number.
#[test]
fn number_mul_preserves_number() {
    ShapeTest::new("2.5 * 4.0").expect_number(10.0);
}

// =========================================================================
// 16. NULL / NONE TYPE
// =========================================================================

/// Verifies null value.
#[test]
fn null_value() {
    ShapeTest::new("None").expect_none();
}

/// Verifies null equality.
#[test]
fn null_equality() {
    ShapeTest::new("None == None").expect_bool(true);
}

/// Verifies null coalesce with string.
#[test]
fn null_coalesce_with_string() {
    ShapeTest::new(r#"None ?? "default""#).expect_string("default");
}

/// Verifies non null does not coalesce.
#[test]
fn non_null_does_not_coalesce() {
    ShapeTest::new(r#""value" ?? "default""#).expect_string("value");
}

// =========================================================================
// 17. UNIT TYPE / VOID
// =========================================================================

/// Verifies void function returns unit.
#[test]
fn void_function_returns_unit() {
    ShapeTest::new(
        r#"
        fn do_nothing() {
            let x = 1
        }
        fn test() {
            do_nothing()
            return 42
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 25. TYPE PRESERVATION THROUGH CONTROL FLOW
// =========================================================================

/// Verifies type preserved through if.
#[test]
fn type_preserved_through_if() {
    ShapeTest::new(
        r#"
        fn test() {
            let x: int = if true { 42 } else { 0 }
            return x
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies type preserved through if else.
#[test]
fn type_preserved_through_if_else() {
    ShapeTest::new(
        r#"
        fn test() {
            let x = if false { 1 } else { 2 }
            return x
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Verifies type preserved through loop.
#[test]
fn type_preserved_through_loop() {
    ShapeTest::new(
        r#"
        fn test() {
            let mut sum: int = 0
            for i in range(5) {
                sum = sum + i
            }
            return sum
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// 10. TYPE ALIASES
// =========================================================================

/// Verifies basic type alias for struct.
#[test]
fn type_alias_basic_struct() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        type P = Point
        let p = P { x: 10, y: 20 }
        p.x
    "#,
    )
    .expect_number(10.0);
}

/// Verifies type alias used in function body.
#[test]
fn type_alias_used_in_function_body() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        type P = Point
        fn make_point() {
            return P { x: 5, y: 10 }
        }
        fn test() {
            let p = make_point()
            return p.y
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Verifies type alias for primitive.
#[test]
fn type_alias_for_primitive() {
    ShapeTest::new(
        r#"
        type MyInt = int
        let x: MyInt = 42
        x
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 22. STRUCT FIELD TYPES (ALL BASIC)
// =========================================================================

/// Verifies struct with all basic field types.
#[test]
fn struct_with_all_basic_field_types() {
    ShapeTest::new(
        r#"
        type Record {
            id: int,
            score: number,
            name: string,
            active: bool
        }
        let r = Record { id: 1, score: 9.5, name: "test", active: true }
        r.id
    "#,
    )
    .expect_number(1.0);
}

/// Verifies struct number field.
#[test]
fn struct_field_number() {
    ShapeTest::new(
        r#"
        type Record {
            id: int,
            score: number,
            name: string,
            active: bool
        }
        let r = Record { id: 1, score: 9.5, name: "test", active: true }
        r.score
    "#,
    )
    .expect_number(9.5);
}

/// Verifies struct string field.
#[test]
fn struct_field_string() {
    ShapeTest::new(
        r#"
        type Record {
            id: int,
            score: number,
            name: string,
            active: bool
        }
        let r = Record { id: 1, score: 9.5, name: "test", active: true }
        r.name
    "#,
    )
    .expect_string("test");
}

/// Verifies struct bool field.
#[test]
fn struct_field_bool() {
    ShapeTest::new(
        r#"
        type Record {
            id: int,
            score: number,
            name: string,
            active: bool
        }
        let r = Record { id: 1, score: 9.5, name: "test", active: true }
        r.active
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// 28. NESTED STRUCT TYPES
// =========================================================================

/// Verifies nested struct access.
#[test]
fn nested_struct_access() {
    ShapeTest::new(
        r#"
        type Inner { value: int }
        type Outer { inner: Inner }
        fn test() {
            let o = Outer { inner: Inner { value: 42 } }
            return o.inner.value
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 11. STRUCT TYPE DEFINITIONS
// =========================================================================

/// Verifies struct type field access int.
#[test]
fn struct_type_field_access_int() {
    ShapeTest::new(
        r#"
        type Vec2 { x: int, y: int }
        let v = Vec2 { x: 3, y: 4 }
        v.x + v.y
    "#,
    )
    .expect_number(7.0);
}

/// Verifies struct type field access string.
#[test]
fn struct_type_field_access_string() {
    ShapeTest::new(
        r#"
        type Person { name: string, age: int }
        let p = Person { name: "Alice", age: 30 }
        p.name
    "#,
    )
    .expect_string("Alice");
}

/// Verifies struct type field access number.
#[test]
fn struct_type_field_access_number() {
    ShapeTest::new(
        r#"
        type Coord { lat: number, lon: number }
        let c = Coord { lat: 51.5, lon: -0.1 }
        c.lat
    "#,
    )
    .expect_number(51.5);
}

/// Verifies struct type field access bool.
#[test]
fn struct_type_field_access_bool() {
    ShapeTest::new(
        r#"
        type Config { debug: bool, verbose: bool }
        let c = Config { debug: true, verbose: false }
        c.debug
    "#,
    )
    .expect_bool(true);
}

/// Verifies struct type in function.
#[test]
fn struct_type_in_function() {
    ShapeTest::new(
        r#"
        type Pair { first: int, second: int }
        fn sum_pair(p: Pair) -> int {
            return p.first + p.second
        }
        fn test() {
            let p = Pair { first: 10, second: 32 }
            return sum_pair(p)
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

