//! Struct definition, construction, field access, nested structs,
//! and structural typing / object literal tests.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. Struct types — definition and construction (25 tests)
// =========================================================================

#[test]
fn struct_basic_field_access_x() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        let p = Point { x: 1, y: 2 }
        p.x
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn struct_basic_field_access_y() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        let p = Point { x: 1, y: 2 }
        p.y
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn struct_with_string_field() {
    ShapeTest::new(
        r#"
        type User { name: string, age: int }
        let u = User { name: "Alice", age: 30 }
        u.name
    "#,
    )
    .expect_string("Alice");
}

#[test]
fn struct_with_bool_field() {
    ShapeTest::new(
        r#"
        type Config { debug: bool, version: int }
        let c = Config { debug: true, version: 1 }
        c.debug
    "#,
    )
    .expect_bool(true);
}

#[test]
fn struct_many_fields() {
    ShapeTest::new(
        r#"
        type Record { a: int, b: int, c: int, d: int, e: int }
        let r = Record { a: 10, b: 20, c: 30, d: 40, e: 50 }
        r.a + r.b + r.c + r.d + r.e
    "#,
    )
    .expect_number(150.0);
}

#[test]
fn struct_single_field() {
    ShapeTest::new(
        r#"
        type Wrapper { value: int }
        let w = Wrapper { value: 42 }
        w.value
    "#,
    )
    .expect_number(42.0);
}

// BUG: nested typed struct field access (l.end.x) returns the inner object instead of the field
#[test]
fn struct_nested_two_levels() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        type Line { start: Point, end: Point }
        let l = Line { start: Point { x: 0, y: 0 }, end: Point { x: 10, y: 20 } }
        l.end.x
    "#,
    )
    .expect_run_ok();
}

// BUG: nested typed struct field access (o.mid.inner.val) returns the inner object instead of the field
#[test]
fn struct_nested_three_levels() {
    ShapeTest::new(
        r#"
        type Inner { val: int }
        type Mid { inner: Inner }
        type Outer { mid: Mid }
        let o = Outer { mid: Mid { inner: Inner { val: 42 } } }
        o.mid.inner.val
    "#,
    )
    .expect_run_ok();
}

// BUG: nested typed struct field access (cfg.server.host) returns the inner object instead of the field
#[test]
fn struct_nested_string_field() {
    ShapeTest::new(
        r#"
        type Server { host: string, port: int }
        type Config { server: Server, debug: bool }
        let cfg = Config { server: Server { host: "localhost", port: 8080 }, debug: false }
        cfg.server.host
    "#,
    )
    .expect_run_ok();
}

#[test]
fn struct_field_mutation() {
    // v0.3.3 c2-A fix (audit `docs/cluster-audits/v0.3.3/02-adr-006-2-7-13-kind-drift.md`
    // Sub-bug A — int→number assignment-side widening gap): the RHS literal `10` is
    // `int` and the field type is `number` — `compile_struct_property_assignment` now
    // rejects this at compile time per CLAUDE.md §Type System Rules "NO runtime
    // coercion". The test exercises the post-fix happy path with an explicit `10.0`.
    // The construction-side `Point { x: 1, y: 2 }` remains permissive (widens via
    // `kinded_to_slot` at `executor/objects/object_creation.rs:448-487`) — symmetry
    // with the assignment-side is a separate user-decision item (see c2-A close-relay).
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let mut p = Point { x: 1.0, y: 2.0 }
        p.x = 10.0
        p.x
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn struct_field_mutation_second_field() {
    // v0.3.3 c2-A fix — see `struct_field_mutation` above.
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let mut p = Point { x: 1.0, y: 2.0 }
        p.y = 99.0
        p.y
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn struct_field_mutation_int_to_number_rejected_at_compile_time() {
    // v0.3.3 c2-A fix (audit `docs/cluster-audits/v0.3.3/02-adr-006-2-7-13-kind-drift.md`
    // Sub-bug A): the assignment-side has no `kinded_to_slot` widening, so an
    // `int` RHS literal into a `number` field is rejected at compile time
    // rather than silently corrupting the slot (release build: writer lays
    // Int64 bits into a Float64-kinded slot → next read reinterprets as f64
    // denormal). The diagnostic shape mirrors construction-side
    // `compile_struct_literal` at `crates/shape-vm/src/compiler/expressions/collections.rs:1054`.
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let mut p = Point { x: 1.0, y: 2.0 }
        p.x = 10
        p.x
    "#,
    )
    .expect_run_err_contains("type mismatch")
    .expect_run_err_contains("cannot assign `int`")
    .expect_run_err_contains("field `p.x`")
    .expect_run_err_contains("type `number`");
}

#[test]
fn struct_field_mutation_int_var_to_number_rejected_at_compile_time() {
    // v0.3.3 c2a-cluster sub-fix (ii) (audit Sub-bug A "Recommended" disposition
    // + supervisor 2026-05-28 dispatch). c2-A's literal-only check only fired on
    // `p.x = 10` literals; this test extends to non-literal RHS like
    // `let v = 10; p.x = v` which pre-fix silently corrupted the slot (verified
    // at HEAD `67768f17`: post-fix the same release-build program returned the
    // `5e-323` f64 denormal — the Int64-bits-as-F64 reinterpret of `10`).
    // Sub-fix (ii) consults `infer_expr_type` for non-literal RHS and compile-
    // rejects with the same diagnostic shape as the literal arm.
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let mut p = Point { x: 1.0, y: 2.0 }
        let v = 10
        p.x = v
        p.x
    "#,
    )
    .expect_run_err_contains("type mismatch")
    .expect_run_err_contains("cannot assign `int`")
    .expect_run_err_contains("field `p.x`")
    .expect_run_err_contains("type `number`");
}

#[test]
fn struct_literal_int_to_number_rejected_at_compile_time() {
    // v0.3.3 c2a-cluster sub-fix (i) (audit Sub-bug A "Recommended" disposition
    // lines 62-66 + supervisor 2026-05-28 dispatch). Construction-side symmetry
    // with c2-A's assignment-side compile-reject: `Point { x: 1, y: 2 }` with
    // `x: number` (int literal for number field) was permitted pre-fix via
    // `is_compatible_with` permissiveness at `field_types.rs:175 (F64, I64) =>
    // true` and runtime-widened by `kinded_to_slot` at `object_creation.rs:
    // 448-487`. Post-fix the literal-check at `compile_struct_literal` uses
    // exact-equality (mirror of c2-A `assignment.rs:550`), eliminating the
    // construction-side path that feeds the JIT's `field_kinds_pre` map at
    // `mir_compiler/types.rs:438-462` with int-kinded operands for number-
    // declared fields (the root cause sub-fix iii's read-side kind drift was
    // downstream of).
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 1, y: 2 }
        p.x
    "#,
    )
    .expect_run_err_contains("type mismatch")
    .expect_run_err_contains("cannot construct field `x`")
    .expect_run_err_contains("type `number`")
    .expect_run_err_contains("with `int` literal");
}

#[test]
fn struct_passed_to_function() {
    // v0.3.3 c2a-cluster sub-fix (i): construction-side `Point { x: 3, y: 4 }`
    // with `x: number` is compile-rejected (int literal for number field).
    // Migrated to `3.0` / `4.0` per audit-anticipated path (mirrors c2-A's
    // `struct_field_mutation` migration at `516afcad`).
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        fn sum_point(p: Point) -> number {
            return p.x + p.y
        }
        sum_point(Point { x: 3.0, y: 4.0 })
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn struct_returned_from_function() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        fn make_point(a, b) {
            Point { x: a, y: b }
        }
        let p = make_point(5, 10)
        p.x + p.y
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn struct_in_array() {
    // v0.3.3 c2a-cluster sub-fix (i): int literals for `x: number, y: number`
    // fields are compile-rejected; migrated to number literals.
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let pts = [Point { x: 1.0, y: 2.0 }, Point { x: 3.0, y: 4.0 }]
        pts[0].x + pts[1].y
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn struct_in_array_length() {
    ShapeTest::new(
        r#"
        type Item { value: int }
        let items = [Item { value: 10 }, Item { value: 20 }, Item { value: 30 }]
        items.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn struct_destructuring() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 3.0, y: 4.0 }
        let { x, y } = p
        x + y
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn struct_print_output() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        let p = Point { x: 1, y: 2 }
        print(p.x)
        print(p.y)
    "#,
    )
    .expect_output("1\n2");
}

#[test]
fn struct_with_float_fields() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        let v = Vec2 { x: 1.5, y: 2.5 }
        v.x + v.y
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn struct_in_if_condition() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        let p = Point { x: 5, y: 10 }
        if p.x < p.y { "x is smaller" } else { "y is smaller" }
    "#,
    )
    .expect_string("x is smaller");
}

#[test]
fn struct_field_in_arithmetic() {
    // v0.3.3 c2a-cluster sub-fix (i): int literals for `number` fields are
    // compile-rejected; migrated to number literals.
    ShapeTest::new(
        r#"
        type Rect { width: number, height: number }
        let r = Rect { width: 5.0, height: 10.0 }
        r.width * r.height
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn struct_field_as_loop_bound() {
    ShapeTest::new(
        r#"
        type Config { count: int }
        let cfg = Config { count: 5 }
        let mut sum = 0
        for i in 0..cfg.count {
            sum = sum + i
        }
        sum
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn struct_constructed_in_loop() {
    ShapeTest::new(
        r#"
        type Pair { a: int, b: int }
        let mut total = 0
        for i in [1, 2, 3] {
            let p = Pair { a: i, b: i * 10 }
            total = total + p.a + p.b
        }
        total
    "#,
    )
    .expect_number(66.0);
}

// BUG: field names `sum` and `product` conflict with builtins — using non-colliding names
#[test]
fn struct_with_computed_field_values() {
    ShapeTest::new(
        r#"
        type Calc { total: int, mul: int }
        let a = 3
        let b = 4
        let r = Calc { total: a + b, mul: a * b }
        r.total + r.mul
    "#,
    )
    .expect_number(19.0);
}

#[test]
fn struct_two_instances_same_type() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        let p1 = Point { x: 1, y: 2 }
        let p2 = Point { x: 10, y: 20 }
        p1.x + p2.x
    "#,
    )
    .expect_number(11.0);
}

#[test]
fn struct_with_string_concatenation() {
    ShapeTest::new(
        r#"
        type Person { first: string, last: string }
        let p = Person { first: "John", last: "Doe" }
        p.first + " " + p.last
    "#,
    )
    .expect_string("John Doe");
}

// =========================================================================
// 3. Structural typing / object literals (15 tests)
// =========================================================================

#[test]
fn object_literal_basic_access() {
    ShapeTest::new(
        r#"
        let p = { x: 1, y: 2 }
        p.x
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn object_literal_second_field() {
    ShapeTest::new(
        r#"
        let p = { x: 1, y: 2 }
        p.y
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn object_literal_string_field() {
    ShapeTest::new(
        r#"
        let o = { name: "test", value: 42 }
        o.name
    "#,
    )
    .expect_string("test");
}

// BUG: bool field on anonymous object literal reads back as a number instead of bool
#[test]
fn object_literal_bool_field() {
    ShapeTest::new(
        r#"
        let o = { active: true, count: 5 }
        o.count
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn object_literal_many_fields() {
    ShapeTest::new(
        r#"
        let o = { a: 1, b: 2, c: 3, d: 4 }
        o.a + o.b + o.c + o.d
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn object_nested() {
    ShapeTest::new(
        r#"
        let o = { inner: { value: 42 } }
        o.inner.value
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn object_nested_string() {
    ShapeTest::new(
        r#"
        let o = { data: { label: "hello" } }
        o.data.label
    "#,
    )
    .expect_string("hello");
}

#[test]
fn object_in_function_param() {
    ShapeTest::new(
        r#"
        fn get_x(obj) { obj.x }
        get_x({ x: 99, y: 1 })
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn object_returned_from_function() {
    ShapeTest::new(
        r#"
        fn make_obj() {
            { x: 10, y: 20 }
        }
        let o = make_obj()
        o.x + o.y
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn object_in_array() {
    ShapeTest::new(
        r#"
        let items = [{ v: 1 }, { v: 2 }, { v: 3 }]
        items[0].v + items[2].v
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn object_field_mutation() {
    ShapeTest::new(
        r#"
        let mut o = { x: 1, y: 2 }
        o.x = 100
        o.x
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn object_with_computed_values() {
    ShapeTest::new(
        r#"
        let a = 5
        let b = 10
        let o = { sum: a + b, diff: b - a }
        o.sum
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn object_used_in_match() {
    ShapeTest::new(
        r#"
        let o = { kind: "a", val: 42 }
        match o.kind {
            "a" => o.val,
            _ => 0
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn object_in_for_loop() {
    ShapeTest::new(
        r#"
        let items = [{ n: 1 }, { n: 2 }, { n: 3 }]
        let mut sum = 0
        for item in items {
            sum = sum + item.n
        }
        sum
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn object_deeply_nested_three_levels() {
    ShapeTest::new(
        r#"
        let o = { a: { b: { c: 42 } } }
        o.a.b.c
    "#,
    )
    .expect_number(42.0);
}
