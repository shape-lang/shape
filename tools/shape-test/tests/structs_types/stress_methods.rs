//! Stress tests for struct methods (extend blocks), struct as function params,
//! struct as return values, struct in closures, and higher-order functions with structs.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 8. OBJECT AS FUNCTION PARAM
// =========================================================================

/// Verifies struct as function parameter.
#[test]
fn struct_as_function_param() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function get_x(p: Point) -> number {
            return p.x
        }
        function test() {
            let p = Point { x: 42.0, y: 0.0 }
            return get_x(p)
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies struct param field arithmetic.
#[test]
fn struct_param_field_arithmetic() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function sum_fields(p: Point) -> number {
            return p.x + p.y
        }
        function test() {
            return sum_fields(Point { x: 3.0, y: 4.0 })
        }
        test()
    "#,
    )
    .expect_number(7.0);
}

/// Verifies struct with multiple params.
#[test]
fn struct_param_multiple_params() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function add_points(a: Point, b: Point) -> number {
            return a.x + b.x + a.y + b.y
        }
        function test() {
            return add_points(Point { x: 1.0, y: 2.0 }, Point { x: 3.0, y: 4.0 })
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// 9. OBJECT AS RETURN VALUE
// =========================================================================

/// Verifies struct as return value.
#[test]
fn struct_as_return_value() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function make_point() -> Point {
            return Point { x: 10.0, y: 20.0 }
        }
        function test() {
            let p = make_point()
            return p.x
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Verifies struct return then field access.
#[test]
fn struct_return_then_field_access() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function origin() -> Point {
            return Point { x: 0.0, y: 0.0 }
        }
        function test() {
            let p = origin()
            return p.x + p.y
        }
        test()
    "#,
    )
    .expect_number(0.0);
}

/// Verifies chained return field access.
#[test]
fn chained_return_field_access() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function make_point(a: number, b: number) -> Point {
            return Point { x: a, y: b }
        }
        function test() {
            return make_point(7.0, 8.0).x
        }
        test()
    "#,
    )
    .expect_number(7.0);
}

// =========================================================================
// 27. STRUCT PASSED THROUGH MULTIPLE FUNCTIONS
// =========================================================================

/// Verifies struct passed through two functions.
#[test]
fn struct_through_two_functions() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function get_x(p: Point) -> number { return p.x }
        function wrap_get_x(p: Point) -> number { return get_x(p) }
        function test() {
            return wrap_get_x(Point { x: 99.0, y: 0.0 })
        }
        test()
    "#,
    )
    .expect_number(99.0);
}

// =========================================================================
// 32. STRUCT LITERAL IN EXPRESSION POSITION
// =========================================================================

/// Verifies struct literal in return.
#[test]
fn struct_literal_in_return() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            return Point { x: 1.0, y: 2.0 }.x
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies struct literal as function argument.
#[test]
fn struct_literal_as_function_argument() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function get_x(p: Point) -> number { return p.x }
        function test() {
            return get_x(Point { x: 55.0, y: 0.0 })
        }
        test()
    "#,
    )
    .expect_number(55.0);
}

// =========================================================================
// 36. TYPE WITH EXTEND BLOCK (methods)
// =========================================================================

/// Verifies extend struct with method.
#[test]
fn extend_struct_with_method() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        extend Point {
            method sum() { self.x + self.y }
        }
        let p = Point { x: 3.0, y: 4.0 }
        p.sum()
    "#,
    )
    .expect_number(7.0);
}

/// Verifies extend struct method with param.
#[test]
fn extend_struct_method_with_param() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        extend Point {
            method scale(factor) { Point { x: self.x * factor, y: self.y * factor } }
        }
        let p = Point { x: 2.0, y: 3.0 }
        let q = p.scale(10.0)
        q.x
    "#,
    )
    .expect_number(20.0);
}

// =========================================================================
// 37. STRUCT FIELD ACCESS CHAINS
// =========================================================================

/// Verifies chain field access on nested struct return.
#[test]
fn chain_field_access_on_nested_struct_return() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        type Box { center: Point }
        function make_box() -> Box {
            return Box { center: Point { x: 42.0, y: 84.0 } }
        }
        function test() {
            return make_box().center.x
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 39. TYPED MERGE + DECOMPOSITION
// =========================================================================

/// Verifies typed merge decomposition.
#[test]
fn typed_merge_decomposition() {
    ShapeTest::new(
        r#"
        type TypeA { x: number, y: number }
        type TypeB { z: number }
        let a = { x: 1 }
        a.y = 2
        let b = { z: 3 }
        let c = a + b
        let (f: TypeA, g: TypeB) = c as (TypeA + TypeB)
        f.x + f.y + g.z
    "#,
    )
    .expect_number(6.0);
}

// =========================================================================
// 40. NESTED TYPE IN FUNCTION
// =========================================================================

/// Verifies nested struct in function.
#[test]
fn nested_struct_in_function() {
    ShapeTest::new(
        r#"
        type Inner { val: int }
        type Outer { inner: Inner }
        function get_inner_val(o: Outer) -> int {
            return o.inner.val
        }
        function test() {
            let o = Outer { inner: Inner { val: 77 } }
            return get_inner_val(o)
        }
        test()
    "#,
    )
    .expect_number(77.0);
}

// =========================================================================
// 61-70. STRUCT INTERACTIONS WITH CLOSURES
// =========================================================================

/// Verifies closure captures struct.
#[test]
fn closure_captures_struct() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 3.0, y: 4.0 }
            let f = || p.x + p.y
            return f()
        }
        test()
    "#,
    )
    .expect_number(7.0);
}

/// Verifies closure returns struct field.
#[test]
fn closure_returns_struct_field() {
    ShapeTest::new(
        r#"
        type Wrapper { val: int }
        function test() {
            let w = Wrapper { val: 42 }
            let get_val = || w.val
            return get_val()
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies higher order function with struct.
#[test]
fn higher_order_function_with_struct() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function apply_to_x(p: Point, f) -> number {
            return f(p.x)
        }
        function test() {
            let p = Point { x: 5.0, y: 0.0 }
            return apply_to_x(p, |n| n * 2.0)
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// Anonymous object edge cases (71-80)
// =========================================================================

/// Verifies anon object single int field.
#[test]
fn anon_object_single_int_field() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { value: 77 }
            return obj.value
        }
        test()
    "#,
    )
    .expect_number(77.0);
}

/// Verifies anon object bool field.
#[test]
fn anon_object_bool_field() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { flag: true }
            return obj.flag
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies anon object with expression values.
#[test]
fn anon_object_with_expression_values() {
    ShapeTest::new(
        r#"
        function test() {
            let a = 10
            let b = 20
            let obj = { sum: a + b, diff: a - b }
            return obj.sum
        }
        test()
    "#,
    )
    .expect_number(30.0);
}

/// Verifies anon object multiple string fields.
#[test]
fn anon_object_multiple_string_fields() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { first: "hello", second: "world" }
            return obj.first + " " + obj.second
        }
        test()
    "#,
    )
    .expect_string("hello world");
}

/// Verifies anon object five fields.
#[test]
fn anon_object_five_fields() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { a: 1, b: 2, c: 3, d: 4, e: 5 }
            return obj.a + obj.b + obj.c + obj.d + obj.e
        }
        test()
    "#,
    )
    .expect_number(15.0);
}

/// Verifies anon nested two levels.
#[test]
fn anon_nested_two_levels() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { outer: { inner: 42 } }
            return obj.outer.inner
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies anon nested three levels string.
#[test]
fn anon_nested_three_levels_string() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { a: { b: { name: "deep" } } }
            return obj.a.b.name
        }
        test()
    "#,
    )
    .expect_string("deep");
}

/// Verifies anon object passed to function.
#[test]
fn anon_object_passed_to_function() {
    ShapeTest::new(
        r#"
        function get_x(obj) {
            return obj.x
        }
        function test() {
            return get_x({ x: 99 })
        }
        test()
    "#,
    )
    .expect_number(99.0);
}

/// Verifies anon object returned from function.
#[test]
fn anon_object_returned_from_function() {
    ShapeTest::new(
        r#"
        function make_obj() {
            return { val: 123 }
        }
        function test() {
            return make_obj().val
        }
        test()
    "#,
    )
    .expect_number(123.0);
}

/// Verifies anon object field access diff.
#[test]
fn anon_object_field_access_diff() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { sum: 30, diff: -10 }
            return obj.diff
        }
        test()
    "#,
    )
    .expect_number(-10.0);
}
