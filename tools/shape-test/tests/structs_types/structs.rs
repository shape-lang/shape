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
fn struct_field_mutation_int_literal_adopts_number() {
    // REBASELINED under THE RULE (user 2026-06-01, numeric-conversion §4 literal
    // adoption). An int LITERAL into a `number` field-mutation context IS the
    // number literal `10.0` — it adopts the field type when losslessly
    // representable, exactly like the construction-side
    // (`struct_literal_int_literal_adopts_number` below) and the scalar
    // `let n: number = 10` binding. The literal is widened to f64 at the
    // assignment producer (`compiler/expressions/assignment.rs`), so the F64 slot
    // gets an f64-kinded value (no §2.7.13 kind drift). Pre-RULE the strict-flip
    // c2-A rejected this as `int` into `number`; that over-strict literal
    // handling is exactly what THE RULE relaxes (the int-VAR form below STAYS
    // rejecting — only a value/variable never silently crosses families).
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let mut p = Point { x: 1.0, y: 2.0 }
        p.x = 10
        p.x
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn struct_field_mutation_int_var_to_number_rejected_at_compile_time() {
    // RULE-ALIGNED (NOT a behavior rebaseline — only the assertion wording is
    // updated). THE RULE (user 2026-06-01, numeric-conversion §5): a value-level
    // `int` VARIABLE into a `number` field is a compile error (int->number is an
    // explicit `as` cast both directions). Only the literal form adopts context
    // (see `struct_field_mutation_int_literal_adopts_number` above). The reject
    // now surfaces from the inference engine's §2 lossless-lattice constraint
    // (`Could not solve type constraints: number is not compatible with int`)
    // rather than the c2a-cluster compiler-side `type mismatch` diagnostic — the
    // assertion is wording-agnostic on the reject.
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let mut p = Point { x: 1.0, y: 2.0 }
        let v = 10
        p.x = v
        p.x
    "#,
    )
    .expect_run_err();
}

#[test]
fn struct_literal_int_literal_adopts_number() {
    // REBASELINED under THE RULE (user 2026-06-01, numeric-conversion §4 literal
    // adoption). `Point { x: 1, y: 2 }` with `x/y: number`: the int literals `1`
    // and `2` are losslessly representable in `number`, so they adopt the field
    // type — they ARE the number literals `1.0`/`2.0`. Construction-side adoption
    // is implemented at `compiler/expressions/collections.rs::
    // int_literal_adopts_field_type` and the runtime widens via `kinded_to_slot`
    // (`object_creation.rs:448-487`). Pre-RULE the strict-flip c2a-cluster
    // rejected this construction with `E0100 ... with int literal`; THE RULE
    // relaxes the over-strict literal handling (the int-VAR construction form
    // stays rejecting — see `struct_passed_to_function`'s `3.0`/`4.0` callers).
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 1, y: 2 }
        p.x
    "#,
    )
    .expect_number(1.0);
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

// =========================================================================
// R3 regression: string fields read out of array-resident TypedObjects
// (strict-flip, content/large.shape SIGABRT/SIGILL heap corruption).
//
// A `String`-typed struct field built from a `StringV2` value (e.g. an
// `Array<string>` loop variable) was stored with `heap_mask` bit clear
// (`kinded_to_slot` did not recognize `StringV2` as heap), while reads
// (`op_get_field_typed`) and field-reference projection (`MakeFieldRef`)
// sourced the carrier from the schema tag (`NativeKind::String`) instead of
// the storage's authoritative `field_kinds[idx]` (`StringV2`). Reading the
// `StringObj` pointer as an `Arc<String>` ran
// `Arc::increment_strong_count::<String>` against the wrong control block,
// corrupting the heap (SIGABRT "corrupted size vs. prev_size"). These tests
// pin the construct-via-push → index-out → multi-read string-field shape
// that reproduced it; under the bug they crashed nondeterministically.
// =========================================================================

#[test]
fn struct_array_push_index_string_field_multiread_no_corruption() {
    ShapeTest::new(
        r#"
        type SuiteStat { suite: string, passed: int, total_ms: int }
        fn aggregate(suite: string) -> SuiteStat {
            return SuiteStat { suite: suite, passed: 2, total_ms: 52 }
        }
        fn stat_row(st: SuiteStat) -> Array<string> {
            let a: string = st.suite
            let b: string = st.suite
            let c: string = f"{st.passed}"
            let row: Array<string> = [a, b, c]
            return row
        }
        let suites: Array<string> = ["auth", "db", "api"]
        let mut stats: Array<SuiteStat> = []
        for s in suites { stats.push(aggregate(s)) }
        let x = stats[0]
        let y = stats[1]
        let z = stats[2]
        let r0: Array<string> = stat_row(x)
        let r1: Array<string> = stat_row(y)
        let r2: Array<string> = stat_row(z)
        f"{r0[0]}/{r1[0]}/{r2[0]}"
    "#,
    )
    .expect_string("auth/db/api");
}

#[test]
fn struct_array_string_field_into_content_table_no_corruption() {
    // The `[row]` array-literal argument must resolve its element kind
    // independently — pre-fix the sibling `headers` literal leaked
    // `pending_variable_typed_array_kind = Some(String)` onto `[row]`,
    // emitting `TypedArrayPushString` against a `Ptr(TypedArray)` element.
    ShapeTest::new(
        r#"
        type SuiteStat { suite: string, passed: int }
        fn agg(s: string) -> SuiteStat { return SuiteStat { suite: s, passed: 2 } }
        let mut stats: Array<SuiteStat> = []
        stats.push(agg("auth"))
        let st = stats[0]
        let row: Array<string> = [st.suite, f"{st.passed}"]
        let t = Content.table(["s", "p"], [row]).border(Border.rounded).to_string()
        t.contains("auth")
    "#,
    )
    .expect_bool(true);
}
