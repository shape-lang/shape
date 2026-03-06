//! Stress tests for struct/type declarations, field types, type aliases,
//! field count variations, and compile-time type errors.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. TYPE DECLARATION -- simple struct
// =========================================================================

/// Verifies simple two-field struct declaration with number fields.
#[test]
fn type_decl_simple_two_fields() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 1.0, y: 2.0 }
        p.x
    "#,
    )
    .expect_number(1.0);
}

/// Verifies struct with int fields.
#[test]
fn type_decl_int_fields() {
    ShapeTest::new(
        r#"
        type Coord { x: int, y: int }
        let c = Coord { x: 10, y: 20 }
        c.y
    "#,
    )
    .expect_number(20.0);
}

/// Verifies struct with string field.
#[test]
fn type_decl_string_field() {
    ShapeTest::new(
        r#"
        type Name { value: string }
        let n = Name { value: "hello" }
        n.value
    "#,
    )
    .expect_string("hello");
}

/// Verifies struct with bool field (true).
#[test]
fn type_decl_bool_field() {
    ShapeTest::new(
        r#"
        type Flag { active: bool }
        let f = Flag { active: true }
        f.active
    "#,
    )
    .expect_bool(true);
}

/// Verifies struct with bool field (false).
#[test]
fn type_decl_bool_field_false() {
    ShapeTest::new(
        r#"
        type Flag { active: bool }
        let f = Flag { active: false }
        f.active
    "#,
    )
    .expect_bool(false);
}

// =========================================================================
// 7. TYPE ALIAS
// =========================================================================

/// Verifies type alias for a struct.
#[test]
fn type_alias_for_struct() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        type P = Point
        let p = P { x: 1, y: 2 }
        p.x
    "#,
    )
    .expect_number(1.0);
}

/// Verifies type alias field access.
#[test]
fn type_alias_for_struct_field_access() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        type P = Point
        let p = P { x: 10, y: 20 }
        p.x + p.y
    "#,
    )
    .expect_number(30.0);
}

// =========================================================================
// 10. MULTIPLE TYPE DEFINITIONS
// =========================================================================

/// Verifies multiple type definitions can coexist.
#[test]
fn multiple_type_defs() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        type Size { w: number, h: number }
        let p = Point { x: 1.0, y: 2.0 }
        let s = Size { w: 3.0, h: 4.0 }
        p.x + s.w
    "#,
    )
    .expect_number(4.0);
}

/// Verifies multiple types used in a function.
#[test]
fn multiple_types_used_in_function() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        type Rect { origin: Point, width: number, height: number }
        function area(r: Rect) -> number {
            return r.width * r.height
        }
        function test() {
            let r = Rect { origin: Point { x: 0.0, y: 0.0 }, width: 5.0, height: 10.0 }
            return area(r)
        }
        test()
    "#,
    )
    .expect_number(50.0);
}

// =========================================================================
// 11. FIELD TYPES -- int, number, string, bool
// =========================================================================

/// Verifies int field type.
#[test]
fn field_type_int() {
    ShapeTest::new(
        r#"
        type Wrapper { val: int }
        let w = Wrapper { val: 100 }
        w.val
    "#,
    )
    .expect_number(100.0);
}

/// Verifies number field type.
#[test]
fn field_type_number() {
    ShapeTest::new(
        r#"
        type Wrapper { val: number }
        let w = Wrapper { val: 3.14 }
        w.val
    "#,
    )
    .expect_number(3.14);
}

/// Verifies string field type.
#[test]
fn field_type_string() {
    ShapeTest::new(
        r#"
        type Wrapper { val: string }
        let w = Wrapper { val: "test" }
        w.val
    "#,
    )
    .expect_string("test");
}

/// Verifies bool field type (true).
#[test]
fn field_type_bool_true() {
    ShapeTest::new(
        r#"
        type Wrapper { val: bool }
        let w = Wrapper { val: true }
        w.val
    "#,
    )
    .expect_bool(true);
}

/// Verifies bool field type (false).
#[test]
fn field_type_bool_false() {
    ShapeTest::new(
        r#"
        type Wrapper { val: bool }
        let w = Wrapper { val: false }
        w.val
    "#,
    )
    .expect_bool(false);
}

// =========================================================================
// 14. FIELD COUNT -- types with 1, 3, 5, 7 fields
// =========================================================================

/// Verifies single-field type.
#[test]
fn single_field_type() {
    ShapeTest::new(
        r#"
        type Single { value: int }
        let s = Single { value: 7 }
        s.value
    "#,
    )
    .expect_number(7.0);
}

/// Verifies three-field type.
#[test]
fn three_field_type() {
    ShapeTest::new(
        r#"
        type Triple { a: int, b: int, c: int }
        let t = Triple { a: 1, b: 2, c: 3 }
        t.a + t.b + t.c
    "#,
    )
    .expect_number(6.0);
}

/// Verifies five-field type.
#[test]
fn five_field_type() {
    ShapeTest::new(
        r#"
        type Quint { a: int, b: int, c: int, d: int, e: int }
        let q = Quint { a: 1, b: 2, c: 3, d: 4, e: 5 }
        q.a + q.b + q.c + q.d + q.e
    "#,
    )
    .expect_number(15.0);
}

/// Verifies seven-field type.
#[test]
fn seven_field_type() {
    ShapeTest::new(
        r#"
        type Big {
            f1: int, f2: int, f3: int, f4: int,
            f5: int, f6: int, f7: int
        }
        function test() {
            let b = Big { f1: 1, f2: 2, f3: 3, f4: 4, f5: 5, f6: 6, f7: 7 }
            return b.f1 + b.f2 + b.f3 + b.f4 + b.f5 + b.f6 + b.f7
        }
        test()
    "#,
    )
    .expect_number(28.0);
}

// =========================================================================
// 13. ARRAY FIELD -- type with array member
// =========================================================================

/// Verifies struct with array field indexing.
#[test]
fn struct_with_array_field() {
    ShapeTest::new(
        r#"
        type Container { items: Array<int> }
        function test() {
            let c = Container { items: [1, 2, 3] }
            return c.items[0]
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies struct with array field length.
#[test]
fn struct_with_array_field_length() {
    ShapeTest::new(
        r#"
        type Container { items: Array<int> }
        function test() {
            let c = Container { items: [10, 20, 30] }
            return c.items.length
        }
        test()
    "#,
    )
    .expect_number(3.0);
}

// =========================================================================
// 19-21. COMPILE ERROR CASES
// =========================================================================

/// Verifies missing field produces compile error.
#[test]
fn missing_field_error() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 3 }
            return p.x
        }
    "#,
    )
    .expect_run_err();
}

/// Verifies missing all fields produces compile error.
#[test]
fn missing_all_fields_error() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { }
            return p
        }
    "#,
    )
    .expect_run_err();
}

/// Verifies extra field produces compile error.
#[test]
fn extra_field_error() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 1, y: 2, z: 3 }
            return p.x
        }
    "#,
    )
    .expect_run_err();
}

/// Verifies unknown type produces compile error.
#[test]
fn unknown_type_error() {
    ShapeTest::new(
        r#"
        function test() {
            let p = Unknown { x: 3 }
            return p.x
        }
    "#,
    )
    .expect_run_err();
}

/// Verifies object multiply is a compile error.
#[test]
fn object_multiply_is_compile_error() {
    ShapeTest::new(
        r#"
        function test() {
            let x = {x: 1}
            return x * 2
        }
    "#,
    )
    .expect_run_err();
}

// =========================================================================
// 33. MULTIPLE FIELD TYPES IN ONE STRUCT
// =========================================================================

/// Verifies mixed field types -- read score.
#[test]
fn struct_mixed_int_number_string_bool() {
    ShapeTest::new(
        r#"
        type Record {
            id: int,
            score: number,
            name: string,
            active: bool
        }
        function test() {
            let r = Record { id: 1, score: 9.5, name: "test", active: true }
            return r.score
        }
        test()
    "#,
    )
    .expect_number(9.5);
}

/// Verifies mixed field types -- read string.
#[test]
fn struct_mixed_read_string() {
    ShapeTest::new(
        r#"
        type Record {
            id: int,
            score: number,
            name: string,
            active: bool
        }
        function test() {
            let r = Record { id: 1, score: 9.5, name: "test", active: true }
            return r.name
        }
        test()
    "#,
    )
    .expect_string("test");
}

/// Verifies mixed field types -- read bool.
#[test]
fn struct_mixed_read_bool() {
    ShapeTest::new(
        r#"
        type Record {
            id: int,
            score: number,
            name: string,
            active: bool
        }
        function test() {
            let r = Record { id: 1, score: 9.5, name: "test", active: true }
            return r.active
        }
        test()
    "#,
    )
    .expect_bool(true);
}

/// Verifies mixed field types -- read int.
#[test]
fn struct_mixed_read_int() {
    ShapeTest::new(
        r#"
        type Record {
            id: int,
            score: number,
            name: string,
            active: bool
        }
        function test() {
            let r = Record { id: 42, score: 9.5, name: "test", active: true }
            return r.id
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// 35. TYPE FIELD ORDER INDEPENDENCE
// =========================================================================

/// Verifies field order different from declaration.
#[test]
fn struct_field_order_different_from_declaration() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { y: 20.0, x: 10.0 }
        p.x
    "#,
    )
    .expect_number(10.0);
}

/// Verifies field order y first.
#[test]
fn struct_field_order_y_first() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { y: 100.0, x: 50.0 }
        p.y
    "#,
    )
    .expect_number(100.0);
}

// =========================================================================
// 22. TYPE METHOD -- .type() on struct instances
// =========================================================================

/// Verifies .type().to_string() on struct instance.
#[test]
fn type_method_on_struct_instance() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 1.0, y: 2.0 }
            return p.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("Point");
}

/// Verifies .type().to_string() on type symbol.
#[test]
fn type_method_on_type_symbol() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        function test() {
            return Point.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("Point");
}

// =========================================================================
// 23. GENERIC STRUCT
// =========================================================================

/// Verifies generic struct with default type param.
#[test]
fn generic_struct_default_param() {
    ShapeTest::new(
        r#"
        type Box<T = int> { value: T }
        function test() {
            let b = Box { value: 42 }
            return b.value
        }
        test()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies generic struct type name with inferred non-default type.
#[test]
fn generic_struct_inferred_type_name() {
    ShapeTest::new(
        r#"
        type MyType<T = int> { x: T }
        function test() {
            let a = MyType { x: 1.0 }
            return a.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("MyType<number>");
}

/// Verifies generic struct type name with default type.
#[test]
fn generic_struct_default_type_name() {
    ShapeTest::new(
        r#"
        type MyType<T = int> { x: T }
        function test() {
            let a = MyType { x: 1 }
            return a.type().to_string()
        }
        test()
    "#,
    )
    .expect_string("MyType");
}

// =========================================================================
// TYPE MISMATCH ERRORS (111-115)
// =========================================================================

/// Verifies decimal for int field produces error.
#[test]
fn type_mismatch_decimal_for_int_field() {
    ShapeTest::new(
        r#"
        type MyType { i: int }
        let b = MyType { i: 10.2D }
    "#,
    )
    .expect_run_err();
}

/// Verifies string for int field produces error.
#[test]
fn type_mismatch_string_for_int_field() {
    ShapeTest::new(
        r#"
        type MyType { val: int }
        let b = MyType { val: "hello" }
    "#,
    )
    .expect_run_err();
}

/// Verifies bool for number field produces error.
#[test]
fn type_mismatch_bool_for_number_field() {
    ShapeTest::new(
        r#"
        type MyType { val: number }
        let b = MyType { val: true }
    "#,
    )
    .expect_run_err();
}

/// Verifies int for string field produces error.
#[test]
fn type_mismatch_int_for_string_field() {
    ShapeTest::new(
        r#"
        type MyType { val: string }
        let b = MyType { val: 42 }
    "#,
    )
    .expect_run_err();
}

/// Verifies string for bool field produces error.
#[test]
fn type_mismatch_string_for_bool_field() {
    ShapeTest::new(
        r#"
        type MyType { val: bool }
        let b = MyType { val: "true" }
    "#,
    )
    .expect_run_err();
}

/// Verifies dynamic spread without known schema fails.
#[test]
fn dynamic_spread_without_known_schema_fails() {
    ShapeTest::new(
        r#"
        fn merge_dynamic(x) {
            let y = { ...x }
            y
        }
    "#,
    )
    .expect_run_err();
}

// =========================================================================
// Various field type combinations (41-50)
// =========================================================================

/// Verifies two string fields.
#[test]
fn struct_two_string_fields() {
    ShapeTest::new(
        r#"
        type Pair { first: string, second: string }
        let p = Pair { first: "hello", second: "world" }
        p.second
    "#,
    )
    .expect_string("world");
}

/// Verifies two bool fields with conditional.
#[test]
fn struct_two_bool_fields() {
    ShapeTest::new(
        r#"
        type Flags { a: bool, b: bool }
        function test() {
            let f = Flags { a: true, b: false }
            if f.a { return 1 } else { return 0 }
        }
        test()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies int and bool fields.
#[test]
fn struct_int_and_bool() {
    ShapeTest::new(
        r#"
        type Item { count: int, active: bool }
        function test() {
            let item = Item { count: 10, active: true }
            if item.active { return item.count } else { return 0 }
        }
        test()
    "#,
    )
    .expect_number(10.0);
}

/// Verifies number and string fields.
#[test]
fn struct_number_and_string() {
    ShapeTest::new(
        r#"
        type Entry { value: number, label: string }
        let e = Entry { value: 3.14, label: "pi" }
        e.label
    "#,
    )
    .expect_string("pi");
}

/// Verifies all number fields.
#[test]
fn struct_all_number_fields() {
    ShapeTest::new(
        r#"
        type Vec3 { x: number, y: number, z: number }
        let v = Vec3 { x: 1.0, y: 2.0, z: 3.0 }
        v.x + v.y + v.z
    "#,
    )
    .expect_number(6.0);
}

/// Verifies all int fields (four).
#[test]
fn struct_all_int_fields_four() {
    ShapeTest::new(
        r#"
        type Quad { a: int, b: int, c: int, d: int }
        let q = Quad { a: 10, b: 20, c: 30, d: 40 }
        q.a + q.b + q.c + q.d
    "#,
    )
    .expect_number(100.0);
}

/// Verifies string + int + bool.
#[test]
fn struct_string_and_int_and_bool() {
    ShapeTest::new(
        r#"
        type User {
            name: string,
            age: int,
            verified: bool
        }
        function test() {
            let u = User { name: "Alice", age: 30, verified: true }
            if u.verified { return u.age } else { return 0 }
        }
        test()
    "#,
    )
    .expect_number(30.0);
}

/// Verifies number and bool.
#[test]
fn struct_number_and_bool() {
    ShapeTest::new(
        r#"
        type Measurement { value: number, valid: bool }
        function test() {
            let m = Measurement { value: 98.6, valid: true }
            if m.valid { return m.value } else { return 0.0 }
        }
        test()
    "#,
    )
    .expect_number(98.6);
}

/// Verifies int and string.
#[test]
fn struct_int_and_string() {
    ShapeTest::new(
        r#"
        type Entry { id: int, name: string }
        let e = Entry { id: 42, name: "item" }
        e.name
    "#,
    )
    .expect_string("item");
}

/// Verifies bool-only struct.
#[test]
fn struct_bool_only() {
    ShapeTest::new(
        r#"
        type Bits { a: bool, b: bool, c: bool }
        let b = Bits { a: true, b: false, c: true }
        b.c
    "#,
    )
    .expect_bool(true);
}
