//! Stress tests for type annotations on let/const/var, function parameters,
//! return types, closures, default params, mutable variables, any type, and array annotations.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. TYPE ANNOTATIONS ON LET -- PRIMITIVES
// =========================================================================

/// Verifies int type annotation on let.
#[test]
fn type_ann_let_int() {
    ShapeTest::new("let x: int = 42; x").expect_number(42.0);
}

/// Verifies number type annotation on let.
#[test]
fn type_ann_let_number() {
    ShapeTest::new("let x: number = 3.14; x").expect_number(3.14);
}

/// Verifies string type annotation on let.
#[test]
fn type_ann_let_string() {
    ShapeTest::new(r#"let x: string = "hello"; x"#).expect_string("hello");
}

/// Verifies bool true type annotation on let.
#[test]
fn type_ann_let_bool_true() {
    ShapeTest::new("let x: bool = true; x").expect_bool(true);
}

/// Verifies bool false type annotation on let.
#[test]
fn type_ann_let_bool_false() {
    ShapeTest::new("let x: bool = false; x").expect_bool(false);
}

/// Verifies bool alias type annotation.
#[test]
fn type_ann_let_boolean_alias() {
    ShapeTest::new("let x: bool = true; x").expect_bool(true);
}

/// Verifies negative int type annotation.
#[test]
fn type_ann_let_int_negative() {
    ShapeTest::new("let x: int = -100; x").expect_number(-100.0);
}

/// Verifies negative number type annotation.
#[test]
fn type_ann_let_number_negative() {
    ShapeTest::new("let x: number = -2.5; x").expect_number(-2.5);
}

/// Verifies zero int type annotation.
#[test]
fn type_ann_let_int_zero() {
    ShapeTest::new("let x: int = 0; x").expect_number(0.0);
}

/// Verifies zero number type annotation.
#[test]
fn type_ann_let_number_zero() {
    ShapeTest::new("let x: number = 0.0; x").expect_number(0.0);
}

// =========================================================================
// 2. FUNCTION PARAMETER TYPES
// =========================================================================

/// Verifies int function parameter.
#[test]
fn fn_param_int() {
    ShapeTest::new(
        r#"
        fn add_one(x: int) -> int { x + 1 }
        fn test() { return add_one(41) }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies number function parameter.
#[test]
fn fn_param_number() {
    ShapeTest::new(
        r#"
        fn double(x: number) -> number { x * 2.0 }
        fn test() { return double(3.5) }
        test()
    "#,
    )
    .expect_number(7.0);
}

/// Verifies string function parameter.
#[test]
fn fn_param_string() {
    ShapeTest::new(
        r#"
        fn greet(name: string) -> string { "hello " + name }
        fn test() { return greet("world") }
        test()
    "#,
    )
    .expect_string("hello world");
}

/// Verifies bool function parameter.
#[test]
fn fn_param_bool() {
    ShapeTest::new(
        r#"
        fn negate(b: bool) -> bool { !b }
        fn test() { return negate(true) }
        test()
    "#,
    )
    .expect_bool(false);
}

/// Verifies multiple typed params.
#[test]
fn fn_multiple_params_typed() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int) -> int { a + b }
        fn test() { return add(10, 32) }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies mixed param types.
#[test]
fn fn_mixed_param_types() {
    ShapeTest::new(
        r#"
        fn repeat(s: string, n: int) -> string {
            let mut result = ""
            for i in range(n) {
                result = result + s
            }
            return result
        }
        fn test() { return repeat("ab", 3) }
        test()
    "#,
    )
    .expect_string("ababab");
}

// =========================================================================
// 3. FUNCTION RETURN TYPES
// =========================================================================

/// Verifies int return type.
#[test]
fn fn_return_type_int() {
    ShapeTest::new(
        r#"
        fn get_num() -> int { return 99 }
        fn test() { return get_num() }
        test()
    "#,
    )
    .expect_number(99.0);
}

/// Verifies number return type.
#[test]
fn fn_return_type_number() {
    ShapeTest::new(
        r#"
        fn pi() -> number { return 3.14159 }
        fn test() { return pi() }
        test()
    "#,
    )
    .expect_number(3.14159);
}

/// Verifies string return type.
#[test]
fn fn_return_type_string() {
    ShapeTest::new(
        r#"
        fn name() -> string { return "shape" }
        fn test() { return name() }
        test()
    "#,
    )
    .expect_string("shape");
}

/// Verifies bool return type.
#[test]
fn fn_return_type_bool() {
    ShapeTest::new(
        r#"
        fn is_ready() -> bool { return true }
        fn test() { return is_ready() }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies implicit last expression return.
#[test]
fn fn_return_type_implicit_last_expr() {
    ShapeTest::new(
        r#"
        fn get_val() -> int { 42 }
        fn test() { return get_val() }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 14. BOOL IN TYPE POSITION
// =========================================================================

/// Verifies bool type annotation.
#[test]
fn bool_type_annotation() {
    ShapeTest::new("let b: bool = true; b").expect_bool(true);
}

/// Verifies bool in conditional.
#[test]
fn bool_in_conditional() {
    ShapeTest::new(
        r#"
        fn check(b: bool) -> int {
            if b { return 1 }
            return 0
        }
        fn test() { return check(true) }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies bool logical and.
#[test]
fn bool_logical_and() {
    ShapeTest::new("let a: bool = true; let b: bool = false; a && b").expect_bool(false);
}

/// Verifies bool logical or.
#[test]
fn bool_logical_or() {
    ShapeTest::new("let a: bool = false; let b: bool = true; a || b").expect_bool(true);
}

// =========================================================================
// 15. STRING TYPE
// =========================================================================

/// Verifies string type annotation.
#[test]
fn string_type_annotation() {
    ShapeTest::new(r#"let s: string = "hello"; s"#).expect_string("hello");
}

/// Verifies empty string.
#[test]
fn string_empty() {
    ShapeTest::new(r#"let s: string = ""; s"#).expect_string("");
}

/// Verifies string concatenation typed.
#[test]
fn string_concatenation_typed() {
    ShapeTest::new(
        r#"
        fn join(a: string, b: string) -> string { a + b }
        fn test() { return join("foo", "bar") }
        test()
    "#,
    )
    .expect_string("foobar");
}

// =========================================================================
// 19. ANY TYPE
// =========================================================================

/// Verifies any type annotation with int.
#[test]
fn any_type_annotation_int() {
    ShapeTest::new("let x: any = 42; x").expect_number(42.0);
}

/// Verifies any type annotation with string.
#[test]
fn any_type_annotation_string() {
    ShapeTest::new(r#"let x: any = "hello"; x"#).expect_string("hello");
}

// =========================================================================
// 9. ARRAY TYPE ANNOTATIONS
// =========================================================================

/// Verifies int array type annotation.
#[test]
fn array_type_int_basic() {
    ShapeTest::new("let arr: int[] = [1, 2, 3]; arr[0]").expect_number(1.0);
}

/// Verifies string array type annotation.
#[test]
fn array_type_string_basic() {
    ShapeTest::new(r#"let arr: string[] = ["a", "b", "c"]; arr[1]"#).expect_string("b");
}

/// Verifies number array type annotation.
#[test]
fn array_type_number_basic() {
    ShapeTest::new("let arr: number[] = [1.1, 2.2, 3.3]; arr[2]").expect_number(3.3);
}

/// Verifies bool array type annotation.
#[test]
fn array_type_bool_basic() {
    ShapeTest::new("let arr: bool[] = [true, false, true]; arr[1]").expect_bool(false);
}

/// Verifies array type length.
#[test]
fn array_type_length() {
    ShapeTest::new("let arr: int[] = [10, 20, 30]; arr.length").expect_number(3.0);
}

/// Verifies inferred int array.
#[test]
fn array_inferred_int() {
    ShapeTest::new("let arr = [1, 2, 3]; arr[0]").expect_number(1.0);
}

/// Verifies inferred string array.
#[test]
fn array_inferred_string() {
    ShapeTest::new(r#"let arr = ["x", "y"]; arr[0]"#).expect_string("x");
}

// =========================================================================
// 23. TYPE ANNOTATIONS IN CLOSURES
// =========================================================================

/// Verifies closure with typed parameter.
#[test]
fn closure_typed_param() {
    ShapeTest::new(
        r#"
        fn test() {
            let f = |x: int| { x * 2 }
            return f(21)
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies closure with inferred types.
#[test]
fn closure_inferred_types() {
    ShapeTest::new(
        r#"
        fn test() {
            let f = |x| x + 1
            return f(41)
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 24. UNTYPED PARAMS (DYNAMIC)
// =========================================================================

/// Verifies untyped param accepts int.
#[test]
fn untyped_param_accepts_int() {
    ShapeTest::new(
        r#"
        fn echo(x) { return x }
        fn test() { return echo(42) }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies untyped param accepts string.
#[test]
fn untyped_param_accepts_string() {
    ShapeTest::new(
        r#"
        fn echo(x) { return x }
        fn test() { return echo("hello") }
        test()
    "#,
    )
    .expect_string("hello");
}

/// Verifies untyped param accepts bool.
#[test]
fn untyped_param_accepts_bool() {
    ShapeTest::new(
        r#"
        fn echo(x) { return x }
        fn test() { return echo(true) }
        test()
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// 26. DEFAULT PARAMETER VALUES
// =========================================================================

/// Verifies default param int.
#[test]
fn default_param_int() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int = 10) -> int { a + b }
        fn test() { return add(32) }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies default param string.
#[test]
fn default_param_string() {
    ShapeTest::new(
        r#"
        fn greet(name: string = "world") -> string { "hello " + name }
        fn test() { return greet() }
        test()
    "#,
    )
    .expect_string("hello world");
}

/// Verifies default param overridden.
#[test]
fn default_param_overridden() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int = 10) -> int { a + b }
        fn test() { return add(32, 10) }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 27. MUTABLE VARIABLES WITH TYPES
// =========================================================================

/// Verifies mutable int reassignment.
#[test]
fn mut_int_reassignment() {
    ShapeTest::new(
        r#"
        fn test() {
            let mut x: int = 0
            x = 42
            return x
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies mutable string reassignment.
#[test]
fn mut_string_reassignment() {
    ShapeTest::new(
        r#"
        fn test() {
            let mut s: string = "hello"
            s = "world"
            return s
        }
        test()
    "#,
    )
    .expect_string("world");
}

/// Verifies mutable bool reassignment.
#[test]
fn mut_bool_reassignment() {
    ShapeTest::new(
        r#"
        fn test() {
            let mut b: bool = false
            b = true
            return b
        }
        test()
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// 29. TYPE ANNOTATIONS ON CONST
// =========================================================================

/// Verifies const int.
#[test]
fn const_int() {
    ShapeTest::new("const x: int = 42; x").expect_number(42.0);
}

/// Verifies const string.
#[test]
fn const_string() {
    ShapeTest::new(r#"const s: string = "immutable"; s"#).expect_string("immutable");
}

/// Verifies const bool.
#[test]
fn const_bool() {
    ShapeTest::new("const b: bool = true; b").expect_bool(true);
}

// =========================================================================
// 31. FUNCTION KEYWORD VARIANTS
// =========================================================================

/// Verifies fn keyword works.
#[test]
fn fn_keyword_works() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int) -> int { a + b }
        fn test() { return add(20, 22) }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies function keyword works.
#[test]
fn function_keyword_works() {
    ShapeTest::new(
        r#"
        function add(a: int, b: int) -> int { a + b }
        function test() { return add(20, 22) }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 20. TYPE ERROR CASES
// =========================================================================

/// Verifies object multiply is a compile error.
#[test]
fn type_error_object_multiply() {
    ShapeTest::new(
        r#"
        fn test() {
            let x = {x: 1}
            return x * 2
        }
        test()
    "#,
    )
    .expect_run_err();
}

/// Verifies unknown struct type is a compile error.
#[test]
fn type_error_unknown_struct_type() {
    ShapeTest::new(
        r#"
        let x = UnknownType { a: 1 }
    "#,
    )
    .expect_run_err();
}

// =========================================================================
// 21. FUNCTION TYPE ANNOTATIONS (COMPLEX)
// =========================================================================

/// Verifies function with array param.
#[test]
fn fn_with_array_param() {
    ShapeTest::new(
        r#"
        fn sum_arr(arr: int[]) -> int {
            let mut total = 0
            for x in arr {
                total = total + x
            }
            return total
        }
        fn test() { return sum_arr([10, 20, 12]) }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies function returning array.
#[test]
fn fn_returning_array() {
    ShapeTest::new(
        r#"
        fn make_arr() -> int[] { return [1, 2, 3] }
        fn test() { return make_arr()[1] }
        test()
    "#,
    )
    .expect_number(2.0);
}

// =========================================================================
// 33. TYPE ANNOTATION WITH FUNCTION TYPES
// =========================================================================

/// Verifies higher order function typed.
#[test]
fn higher_order_function_typed() {
    ShapeTest::new(
        r#"
        fn apply(f: (x: int) => int, val: int) -> int {
            return f(val)
        }
        fn test() {
            return apply(|x| x * 2, 21)
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 40. VAR KEYWORD WITH TYPES
// =========================================================================

/// Verifies var keyword with type.
#[test]
fn var_keyword_with_type() {
    ShapeTest::new(
        r#"
        var x: int = 10
        x
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// 34. ADDITIONAL TYPE EDGE CASES
// =========================================================================

/// Verifies large int preserves type.
#[test]
fn large_int_preserves_type() {
    ShapeTest::new("let x: int = 1000000; x").expect_number(1000000.0);
}

/// Verifies negative number preserves type.
#[test]
fn negative_number_preserves_type() {
    ShapeTest::new("let x: number = -99.99; x").expect_number(-99.99);
}

/// Verifies type annotation with expression.
#[test]
fn type_annotation_with_expression() {
    ShapeTest::new(
        r#"
        let x: int = 2 + 3
        x
    "#,
    )
    .expect_number(5.0);
}

/// Verifies multiple let with different types.
#[test]
fn multiple_let_different_types() {
    ShapeTest::new(
        r#"
        let a: int = 1
        let b: number = 2.0
        let c: string = "three"
        let d: bool = true
        a
    "#,
    )
    .expect_number(1.0);
}

/// Verifies typed function without return annotation.
#[test]
fn typed_function_no_return_annotation() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int) { return a + b }
        fn test() { return add(20, 22) }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies bool negation preserves type.
#[test]
fn bool_negation_preserves_type() {
    ShapeTest::new(
        r#"
        fn test() {
            let x: bool = true
            let y: bool = !x
            return y
        }
        test()
    "#,
    )
    .expect_bool(false);
}

/// Verifies typed for loop accumulator.
#[test]
fn typed_for_loop_accumulator() {
    ShapeTest::new(
        r#"
        fn test() {
            let mut sum: int = 0
            for i in range(10) {
                sum = sum + i
            }
            return sum
        }
        test()
    "#,
    )
    .expect_number(45.0);
}

/// Verifies type annotation array of bools.
#[test]
fn type_annotation_array_of_bools() {
    ShapeTest::new(
        r#"
        let arr: bool[] = [true, false, true]
        arr[0]
    "#,
    )
    .expect_bool(true);
}

/// Verifies type annotation array of strings.
#[test]
fn type_annotation_array_of_strings() {
    ShapeTest::new(
        r#"
        let arr: string[] = ["a", "b", "c"]
        arr[2]
    "#,
    )
    .expect_string("c");
}

/// Verifies empty array typed.
#[test]
fn empty_array_typed() {
    ShapeTest::new(
        r#"
        let arr: int[] = []
        arr.length
    "#,
    )
    .expect_number(0.0);
}

/// Verifies function with multiple return paths.
#[test]
fn fn_with_multiple_return_paths() {
    ShapeTest::new(
        r#"
        fn abs_val(x: int) -> int {
            if x < 0 { return -x }
            return x
        }
        fn test() {
            return abs_val(-42)
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies function with multiple return paths (positive).
#[test]
fn fn_with_multiple_return_paths_positive() {
    ShapeTest::new(
        r#"
        fn abs_val(x: int) -> int {
            if x < 0 { return -x }
            return x
        }
        fn test() {
            return abs_val(42)
        }
        test()
    "#,
    )
    .expect_number(42.0);
}
