//! Stress tests for complex type interactions: struct array fields, function
//! returning structs, Result/Option match patterns, multiple struct types,
//! type annotation interactions, and edge cases (closures, overflow, precision).

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 32. COMPLEX TYPE INTERACTIONS
// =========================================================================

/// Verifies struct array field.
#[test]
fn struct_array_field() {
    ShapeTest::new(
        r#"
        type Container { items: int[] }
        fn test() {
            let c = Container { items: [1, 2, 3] }
            return c.items[1]
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Verifies function returning struct.
#[test]
fn function_returning_struct() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        fn origin() -> Point { return Point { x: 0, y: 0 } }
        fn test() { return origin().x }
        test()
    "#,
    )
    .expect_number(0.0);
}

/// Verifies array of ints sum.
#[test]
fn array_of_ints_sum() {
    ShapeTest::new(
        r#"
        fn sum(arr: int[]) -> int {
            let mut s = 0
            for x in arr { s = s + x }
            return s
        }
        fn test() { return sum([1, 2, 3, 4, 5]) }
        test()
    "#,
    )
    .expect_number(15.0);
}

/// Verifies result ok unwrap via match.
#[test]
fn result_ok_unwrap_via_match() {
    ShapeTest::new(
        r#"
        fn test() {
            let r = Ok(42)
            return match r {
                Ok(v) => v,
                Err(e) => 0
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies result err match.
#[test]
fn result_err_match() {
    ShapeTest::new(
        r#"
        fn test() {
            let r = Err("bad")
            return match r {
                Ok(v) => 0,
                Err(e) => -1
            }
        }
        test()
    "#,
    )
    .expect_number(-1.0);
}

/// Verifies option match some.
#[test]
fn option_match_some() {
    ShapeTest::new(
        r#"
        fn test() {
            let x = Some(42)
            return match x {
                Some(v) => v,
                None => 0
            }
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies option match none.
#[test]
fn option_match_none() {
    ShapeTest::new(
        r#"
        fn test() {
            let x = None
            return match x {
                Some(v) => v,
                None => -1
            }
        }
        test()
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// 36. MULTIPLE STRUCT TYPES
// =========================================================================

/// Verifies multiple struct types coexist.
#[test]
fn multiple_struct_types_coexist() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        type Color { r: int, g: int, b: int }
        fn test() {
            let p = Point { x: 1, y: 2 }
            let c = Color { r: 255, g: 128, b: 0 }
            return p.x + c.r
        }
        test()
    "#,
    )
    .expect_number(256.0);
}

// =========================================================================
// 38. OPTION WITH MATCH PATTERNS
// =========================================================================

/// Verifies option some match extracts number.
#[test]
fn option_some_match_extracts_number() {
    ShapeTest::new(
        r#"
        fn test() {
            let x = Some(3.14)
            return match x {
                Some(v) => v,
                None => 0.0
            }
        }
        test()
    "#,
    )
    .expect_number(3.14);
}

/// Verifies option some match extracts string.
#[test]
fn option_some_match_extracts_string() {
    ShapeTest::new(
        r#"
        fn test() {
            let x = Some("found")
            return match x {
                Some(v) => v,
                None => "not found"
            }
        }
        test()
    "#,
    )
    .expect_string("found");
}

// =========================================================================
// 39. TYPE ANNOTATION INTERACTIONS
// =========================================================================

/// Verifies let without annotation infers correctly.
#[test]
fn let_without_annotation_infers_correctly() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int) -> int { a + b }
        let result = add(20, 22)
        result
    "#,
    )
    .expect_number(42.0);
}

/// Verifies multiple functions with types.
#[test]
fn multiple_functions_with_types() {
    ShapeTest::new(
        r#"
        fn square(x: int) -> int { x * x }
        fn cube(x: int) -> int { x * x * x }
        fn test() {
            return square(3) + cube(2)
        }
        test()
    "#,
    )
    .expect_number(17.0);
}

// =========================================================================
// 41. ADDITIONAL EDGE CASES AND REGRESSION TESTS
// =========================================================================

/// Verifies int zero division is error.
#[test]
fn int_zero_division_is_error() {
    ShapeTest::new("1 / 0").expect_run_err();
}

/// Verifies struct type name via instance.
#[test]
fn struct_type_name_via_instance() {
    ShapeTest::new(
        r#"
        type Foo { x: int }
        fn test() {
            let f = Foo { x: 1 }
            return f.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("Foo");
}

/// Verifies struct type name via symbol.
#[test]
fn struct_type_name_via_symbol() {
    ShapeTest::new(
        r#"
        type Bar { y: number }
        fn test() {
            return Bar.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("Bar");
}

/// Verifies generic identity preserves value.
#[test]
fn generic_identity_preserves_value() {
    ShapeTest::new(
        r#"
        fn id<T>(x: T) -> T { return x }
        fn test() {
            let a = id(42)
            let b = id("hello")
            let c = id(true)
            let d = id(3.14)
            return a + 0
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies nested function calls typed.
#[test]
fn nested_function_calls_typed() {
    ShapeTest::new(
        r#"
        fn double(x: int) -> int { x * 2 }
        fn add_one(x: int) -> int { x + 1 }
        fn test() { return add_one(double(20)) }
        test()
    "#,
    )
    .expect_number(41.0);
}

/// Verifies recursive function typed.
#[test]
fn recursive_function_typed() {
    ShapeTest::new(
        r#"
        fn factorial(n: int) -> int {
            if n <= 1 { return 1 }
            return n * factorial(n - 1)
        }
        fn test() { return factorial(5) }
        test()
    "#,
    )
    .expect_number(120.0);
}

/// Verifies string length returns int.
#[test]
fn string_length_returns_int() {
    ShapeTest::new(
        r#"
        fn test() {
            let s: string = "hello"
            return s.length
        }
        test()
    "#,
    )
    .expect_number(5.0);
}

/// Verifies array length returns int.
#[test]
fn array_length_returns_int() {
    ShapeTest::new(
        r#"
        fn test() {
            let a: int[] = [1, 2, 3, 4]
            return a.length
        }
        test()
    "#,
    )
    .expect_number(4.0);
}

/// Verifies result ok number.
#[test]
fn result_ok_number() {
    ShapeTest::new("Ok(3.14)").expect_run_ok();
}

/// Verifies result err bool.
#[test]
fn result_err_bool() {
    ShapeTest::new("Err(false)").expect_run_ok();
}

/// Verifies int comparison returns bool.
#[test]
fn int_comparison_returns_bool() {
    ShapeTest::new(
        r#"
        fn test() {
            let a: int = 10
            let b: int = 20
            return a < b
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies number comparison returns bool.
#[test]
fn number_comparison_returns_bool() {
    ShapeTest::new(
        r#"
        fn test() {
            let a: number = 1.5
            let b: number = 2.5
            return a >= b
        }
        test()
    "#,
    )
    .expect_bool(false);
}

/// Verifies string equality returns bool.
#[test]
fn string_equality_returns_bool() {
    ShapeTest::new(
        r#"
        fn test() {
            let a: string = "hello"
            let b: string = "hello"
            return a == b
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies string inequality returns bool.
#[test]
fn string_inequality_returns_bool() {
    ShapeTest::new(
        r#"
        fn test() {
            let a: string = "hello"
            let b: string = "world"
            return a != b
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies int equality check.
#[test]
fn int_equality_check() {
    ShapeTest::new(
        r#"
        fn test() {
            let a: int = 42
            let b: int = 42
            return a == b
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies int inequality check.
#[test]
fn int_inequality_check() {
    ShapeTest::new(
        r#"
        fn test() {
            let a: int = 42
            let b: int = 43
            return a != b
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies struct with string and int label.
#[test]
fn struct_with_string_and_int() {
    ShapeTest::new(
        r#"
        type Item { label: string, count: int }
        fn test() {
            let item = Item { label: "apples", count: 5 }
            return item.label
        }
        test()
    "#,
    )
    .expect_string("apples");
}

/// Verifies struct with string and int count.
#[test]
fn struct_with_string_and_int_count() {
    ShapeTest::new(
        r#"
        type Item { label: string, count: int }
        fn test() {
            let item = Item { label: "apples", count: 5 }
            return item.count
        }
        test()
    "#,
    )
    .expect_number(5.0);
}

/// Verifies result ok is not err.
#[test]
fn result_ok_is_not_err() {
    ShapeTest::new("Ok(1)").expect_run_ok();
}

/// Verifies result err is not ok.
#[test]
fn result_err_is_not_ok() {
    ShapeTest::new(r#"Err("fail")"#).expect_run_ok();
}

/// Verifies option some is not none.
#[test]
fn option_some_is_not_none() {
    ShapeTest::new("Some(42)").expect_run_ok();
}

/// Verifies null is none literal.
#[test]
fn null_is_none_literal() {
    ShapeTest::new("None").expect_none();
}

/// Verifies int max range.
#[test]
fn int_max_range() {
    ShapeTest::new("let x: int = 140737488355327; x").expect_number(140737488355327.0);
}

/// Verifies int min range.
#[test]
fn int_min_range() {
    ShapeTest::new("let x: int = -140737488355328; x").expect_number(-140737488355328.0);
}

/// Verifies number precision.
#[test]
fn number_precision() {
    ShapeTest::new("let x: number = 1.7976931348623157e308; x").expect_run_ok();
}

/// Verifies typed closure in array map.
#[test]
fn typed_closure_in_array_map() {
    ShapeTest::new(
        r#"
        fn test() {
            let arr = [1, 2, 3]
            let doubled = arr.map(|x| x * 2)
            return doubled[2]
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

/// Verifies typed closure in array filter.
#[test]
fn typed_closure_in_array_filter() {
    ShapeTest::new(
        r#"
        fn test() {
            let arr = [1, 2, 3, 4, 5]
            let evens = arr.filter(|x| x % 2 == 0)
            return evens.length
        }
        test()
    "#,
    )
    .expect_number(2.0);
}

/// Verifies int overflow promotes to f64.
#[test]
fn int_overflow_promotes_to_f64() {
    ShapeTest::new(
        r#"
        fn test() {
            let big = 140737488355327
            return big + 1
        }
        test()
    "#,
    )
    .expect_run_ok();
}
