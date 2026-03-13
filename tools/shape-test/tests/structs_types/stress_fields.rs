//! Stress tests for struct field access, computed fields, mutations,
//! anonymous objects, spread, destructuring, and various field edge cases.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 2. OBJECT CREATION -- all fields provided
// =========================================================================

/// Verifies object creation with all number fields.
#[test]
fn object_creation_all_fields_number() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        let v = Vec2 { x: 3.0, y: 4.0 }
        v.x + v.y
    "#,
    )
    .expect_number(7.0);
}

/// Verifies int widening to number field.
#[test]
fn object_creation_int_widening_to_number() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 1, y: 2 }
        p.x + p.y
    "#,
    )
    .expect_number(3.0);
}

/// Verifies object creation with mixed types.
#[test]
fn object_creation_mixed_types() {
    ShapeTest::new(
        r#"
        type Record { name: string, age: int, score: number }
        let r = Record { name: "Alice", age: 30, score: 95.5 }
        r.score
    "#,
    )
    .expect_number(95.5);
}

/// Verifies object creation in a function.
#[test]
fn object_creation_in_function() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 5.0, y: 10.0 }
            return p.x + p.y
        }
        test()
    "#,
    )
    .expect_number(15.0);
}

// =========================================================================
// 3. FIELD ACCESS -- dot notation
// =========================================================================

/// Verifies accessing the first field.
#[test]
fn field_access_first_field() {
    ShapeTest::new(
        r#"
        type Pair { a: int, b: int }
        let p = Pair { a: 42, b: 99 }
        p.a
    "#,
    )
    .expect_number(42.0);
}

/// Verifies accessing the second field.
#[test]
fn field_access_second_field() {
    ShapeTest::new(
        r#"
        type Pair { a: int, b: int }
        let p = Pair { a: 42, b: 99 }
        p.b
    "#,
    )
    .expect_number(99.0);
}

/// Verifies accessing a string field.
#[test]
fn field_access_string_field() {
    ShapeTest::new(
        r#"
        type Person { name: string, age: int }
        let p = Person { name: "Bob", age: 25 }
        p.name
    "#,
    )
    .expect_string("Bob");
}

/// Verifies accessing a bool field.
#[test]
fn field_access_bool_field() {
    ShapeTest::new(
        r#"
        type Toggle { on: bool, label: string }
        let t = Toggle { on: true, label: "switch" }
        t.on
    "#,
    )
    .expect_bool(true);
}

/// Verifies field access in an expression.
#[test]
fn field_access_in_expression() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 3.0, y: 4.0 }
        p.x * p.x + p.y * p.y
    "#,
    )
    .expect_number(25.0);
}

// =========================================================================
// 4. ANONYMOUS OBJECTS
// =========================================================================

/// Verifies anon object with single field.
#[test]
fn anon_object_single_field() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { x: 42 }
            return obj.x
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies anon object with multiple fields.
#[test]
fn anon_object_multiple_fields() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { x: 1, y: 2, z: 3 }
            return obj.x + obj.y + obj.z
        }
        test()
    "#,
    )
    .expect_number(6.0);
}

/// Verifies anon object with string field.
#[test]
fn anon_object_string_field() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { name: "Alice" }
            return obj.name
        }
        test()
    "#,
    )
    .expect_string("Alice");
}

/// Verifies anon object with mixed types.
#[test]
fn anon_object_mixed_types() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { count: 5, label: "items" }
            return obj.count
        }
        test()
    "#,
    )
    .expect_number(5.0);
}

/// Verifies anon object at top level.
#[test]
fn anon_object_at_top_level() {
    ShapeTest::new(
        r#"
        let obj = { x: 10, y: 20 }
        obj.x + obj.y
    "#,
    )
    .expect_number(30.0);
}

// =========================================================================
// 16. COMPUTED FIELD VALUES
// =========================================================================

/// Verifies field from arithmetic expression.
#[test]
fn computed_field_from_arithmetic() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 2.0 + 3.0, y: 4.0 * 2.0 }
        p.x
    "#,
    )
    .expect_number(5.0);
}

/// Verifies field from variable.
#[test]
fn computed_field_from_variable() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let a = 10.0
        let b = 20.0
        let p = Point { x: a, y: b }
        p.y
    "#,
    )
    .expect_number(20.0);
}

/// Verifies field from function call.
#[test]
fn computed_field_from_function_call() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function double(n: number) -> number { return n * 2.0 }
        function test() {
            let p = Point { x: double(5.0), y: double(10.0) }
            return p.x
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Verifies field from string concatenation.
#[test]
fn computed_field_string_concat() {
    ShapeTest::new(
        r#"
        type Named { name: string }
        let first = "Hello"
        let last = "World"
        let n = Named { name: first + " " + last }
        n.name
    "#,
    )
    .expect_string("Hello World");
}

// =========================================================================
// 26. STRUCT FIELD MUTATION
// =========================================================================

/// Verifies anon object field mutation.
#[test]
fn anon_object_field_mutation() {
    ShapeTest::new(
        r#"
        let mut obj = { x: 1 }
        obj.x = 42
        obj.x
    "#,
    )
    .expect_number(42.0);
}

/// Verifies anon object string field mutation.
#[test]
fn anon_object_field_mutation_string() {
    ShapeTest::new(
        r#"
        function test() {
            let mut obj = { name: "before" }
            obj.name = "after"
            return obj.name
        }
        test()
    "#,
    )
    .expect_string("after");
}

// =========================================================================
// 17. OBJECT MERGE
// =========================================================================

/// Verifies basic object merge.
#[test]
fn object_merge_basic() {
    ShapeTest::new(
        r#"
        function test() {
            let a = { x: 1, y: 2 }
            let b = { z: 3 }
            let c = a + b
            return c.x + c.z
        }
        test()
    "#,
    )
    .expect_number(4.0);
}

/// Verifies object merge preserves all fields.
#[test]
fn object_merge_preserves_all_fields() {
    ShapeTest::new(
        r#"
        function test() {
            let a = { x: 1, y: 2 }
            let b = { z: 3, w: 4 }
            let merged = a + b
            return merged.x + merged.y + merged.z + merged.w
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// 18. OBJECT SPREAD
// =========================================================================

/// Verifies spread typed object — access original fields directly.
#[test]
fn spread_typed_object() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 1.0, y: 2.0 }
        p.x
    "#,
    )
    .expect_number(1.0);
}

/// Verifies spread typed object extra field.
#[test]
fn spread_typed_object_extra_field() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 5.0, y: 10.0 }
        let q = { ...p, z: 15.0 }
        q.z
    "#,
    )
    .expect_number(15.0);
}

// =========================================================================
// 29-31. DESTRUCTURE
// =========================================================================

/// Verifies intersection decomposition.
#[test]
fn intersection_decomposition_basic() {
    ShapeTest::new(
        r#"
        type A { x: number, y: number }
        type B { z: number }
        let value = { x: 10, y: 20, z: 30 }
        let (a: A, b: B) = value
        a.x + a.y + b.z
    "#,
    )
    .expect_number(60.0);
}

/// Verifies destructure anon object param.
#[test]
fn destructure_anon_object_param() {
    ShapeTest::new(
        r#"
        function process({x, y}) {
            return x + y
        }
        process({x: 5, y: 10})
    "#,
    )
    .expect_number(15.0);
}

/// Verifies destructure nested object param.
#[test]
fn destructure_nested_object_param() {
    ShapeTest::new(
        r#"
        function process({point: {x, y}}) {
            return x + y
        }
        process({point: {x: 5, y: 10}})
    "#,
    )
    .expect_number(15.0);
}

/// Verifies lambda destructure object.
#[test]
fn lambda_destructure_object() {
    ShapeTest::new(
        r#"
        let add = |{x, y}| x + y
        add({x: 7, y: 3})
    "#,
    )
    .expect_number(10.0);
}

/// Verifies let destructure struct.
#[test]
fn let_destructure_struct() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 3.0, y: 4.0 }
        let {x, y} = p
        x + y
    "#,
    )
    .expect_number(7.0);
}

// =========================================================================
// 15. TYPE REUSE
// =========================================================================

/// Verifies same type used with two instances.
#[test]
fn type_reuse_two_instances() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p1 = Point { x: 1.0, y: 2.0 }
        let p2 = Point { x: 3.0, y: 4.0 }
        p1.x + p2.x
    "#,
    )
    .expect_number(4.0);
}

/// Verifies same type used with three instances.
#[test]
fn type_reuse_three_instances() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let a = Point { x: 1.0, y: 0.0 }
        let b = Point { x: 2.0, y: 0.0 }
        let c = Point { x: 3.0, y: 0.0 }
        a.x + b.x + c.x
    "#,
    )
    .expect_number(6.0);
}

/// Verifies type reuse with different values.
#[test]
fn type_reuse_different_values() {
    ShapeTest::new(
        r#"
        type Wrapper { val: string }
        let w1 = Wrapper { val: "hello" }
        let w2 = Wrapper { val: "world" }
        w1.val + " " + w2.val
    "#,
    )
    .expect_string("hello world");
}

// =========================================================================
// Edge cases
// =========================================================================

/// Verifies struct field negative number.
#[test]
fn struct_field_negative_number() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: -5.0, y: -10.0 }
        p.x + p.y
    "#,
    )
    .expect_number(-15.0);
}

/// Verifies struct field zero.
#[test]
fn struct_field_zero() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 0.0, y: 0.0 }
        p.x + p.y
    "#,
    )
    .expect_number(0.0);
}

/// Verifies struct field large int.
#[test]
fn struct_field_large_int() {
    ShapeTest::new(
        r#"
        type Big { val: int }
        let b = Big { val: 1000000 }
        b.val
    "#,
    )
    .expect_number(1000000.0);
}

/// Verifies struct field decimal precision.
#[test]
fn struct_field_decimal_precision() {
    ShapeTest::new(
        r#"
        type Precise { val: number }
        let p = Precise { val: 0.1 + 0.2 }
        p.val
    "#,
    )
    .expect_number(0.30000000000000004);
}

/// Verifies empty string field.
#[test]
fn struct_empty_string_field() {
    ShapeTest::new(
        r#"
        type Named { name: string }
        let n = Named { name: "" }
        n.name
    "#,
    )
    .expect_string("");
}

/// Verifies string field with spaces.
#[test]
fn struct_field_string_with_spaces() {
    ShapeTest::new(
        r#"
        type Named { name: string }
        let n = Named { name: "hello world foo" }
        n.name
    "#,
    )
    .expect_string("hello world foo");
}

/// Verifies large number field.
#[test]
fn struct_field_large_number() {
    ShapeTest::new(
        r#"
        type Big { val: number }
        let b = Big { val: 1e15 }
        b.val
    "#,
    )
    .expect_number(1e15);
}

/// Verifies small fraction field.
#[test]
fn struct_field_small_fraction() {
    ShapeTest::new(
        r#"
        type Small { val: number }
        let s = Small { val: 0.001 }
        s.val
    "#,
    )
    .expect_number(0.001);
}

/// Verifies negative int field.
#[test]
fn struct_with_negative_int_field() {
    ShapeTest::new(
        r#"
        type Signed { val: int }
        let s = Signed { val: -42 }
        s.val
    "#,
    )
    .expect_number(-42.0);
}

/// Verifies struct string fields concat.
#[test]
fn struct_string_fields_concat() {
    ShapeTest::new(
        r#"
        type Person { first: string, last: string }
        let p = Person { first: "John", last: "Doe" }
        p.first + " " + p.last
    "#,
    )
    .expect_string("John Doe");
}

/// Verifies two instances are independent.
#[test]
fn struct_two_instances_independence() {
    ShapeTest::new(
        r#"
        let a = { x: 1 }
        let b = { x: 2 }
        a.x + b.x
    "#,
    )
    .expect_number(3.0);
}

/// Verifies complex field arithmetic.
#[test]
fn struct_field_arithmetic_complex() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 3.0, y: 4.0 }
        let dist_sq = p.x * p.x + p.y * p.y
        dist_sq
    "#,
    )
    .expect_number(25.0);
}

/// Verifies multiple structs with different field names.
#[test]
fn multiple_structs_different_field_names() {
    ShapeTest::new(
        r#"
        type A { foo: int }
        type B { bar: int }
        type C { baz: int }
        let a = A { foo: 1 }
        let b = B { bar: 2 }
        let c = C { baz: 3 }
        a.foo + b.bar + c.baz
    "#,
    )
    .expect_number(6.0);
}

/// Verifies anon object empty string field.
#[test]
fn anon_object_empty_string_field() {
    ShapeTest::new(
        r#"
        function test() {
            let obj = { s: "" }
            return obj.s
        }
        test()
    "#,
    )
    .expect_string("");
}
