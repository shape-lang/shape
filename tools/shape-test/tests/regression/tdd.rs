use shape_test::shape_test::ShapeTest;

// BUG-1: Bare Some/None patterns should work on untyped variables
#[test]
fn bug1_bare_none_match() {
    ShapeTest::new(
        r#"
        let x = Some(42)
        match x { Some(v) => v, None => -1 }
    "#,
    )
    .expect_number(42.0);
}

// BUG-2: Chained calls f(a)(b)
#[test]
fn bug2_chained_call() {
    ShapeTest::new(
        r#"
        fn adder(a) { |b| a + b }
        adder(10)(5)
    "#,
    )
    .expect_number(15.0);
}

// BUG-3: Mutable capture propagates to outer scope
#[test]
fn bug3_mutable_capture_propagates() {
    ShapeTest::new(
        r#"
        let mut count = 0
        let inc = || { count = count + 1; count }
        inc()
        inc()
        count
    "#,
    )
    .expect_number(2.0);
}

// BUG-4: Module member access via ShapeEngine
#[test]
fn bug4_module_member_access() {
    ShapeTest::new(
        r#"
        mod math { pub fn add(a, b) { a + b } }
        math::add(1, 2)
    "#,
    )
    .expect_number(3.0);
}

// BUG-5: Named fn as HOF argument
#[test]
fn bug5_named_fn_as_argument() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        fn apply(f, x) { f(x) }
        apply(double, 21)
    "#,
    )
    .expect_number(42.0);
}

// BUG-6: Closure captures fn-local let variables
#[test]
fn bug6_closure_captures_local_let() {
    ShapeTest::new(
        r#"
        fn make_adder() -> (int) -> int {
            let base: int = 10
            let f = |x: int| { base + x }
            f
        }
        let f = make_adder()
        f(5)
    "#,
    )
    .expect_number(15.0);
}

// BUG-7: Grandparent scope capture
#[test]
fn bug7_grandparent_capture() {
    ShapeTest::new(
        r#"
        let x = 100
        let outer = || {
            |y| x + y
        }
        let f = outer()
        f(42)
    "#,
    )
    .expect_number(142.0);
}

// BUG-8: break expr propagates loop value
#[test]
fn bug8_break_value() {
    ShapeTest::new(
        r#"
        let result = loop { break 42 }
        result
    "#,
    )
    .expect_number(42.0);
}

// BUG-9: const compound assignment errors
#[test]
fn bug9_const_compound_assign() {
    ShapeTest::new(
        r#"
        const C = 10
        C += 1
    "#,
    )
    .expect_run_err_contains("const");
}

// BUG-10: Nested struct field mutation persists
#[test]
fn bug10_nested_field_mutation() {
    ShapeTest::new(
        r#"
        type Inner { val: int }
        type Outer { data: Inner }
        let mut o = Outer { data: Inner { val: 1 } }
        o.data.val = 42
        o.data.val
    "#,
    )
    .expect_number(42.0);
}

// BUG-11: push through & ref propagates
#[test]
fn bug11_push_through_ref() {
    ShapeTest::new(
        r#"
        fn add_item(items: &mut Array<int>, item: int) { items.push(item) }
        let mut items: Array<int> = []
        add_item(&mut items, 1)
        add_item(&mut items, 2)
        items.len()
    "#,
    )
    .expect_number(2.0);
}

// BUG-12: const passed by exclusive & ref errors
#[test]
fn bug12_const_exclusive_ref() {
    ShapeTest::new(
        r#"
        fn inc(&x) { x = x + 1 }
        const C = 5
        inc(&C)
    "#,
    )
    .expect_run_err_contains("const");
}

// BUG-13: Field names don't collide with builtins
#[test]
fn bug13_field_name_sum() {
    ShapeTest::new(
        r#"
        type Stats { sum: number, product: number }
        let s = Stats { sum: 10.0, product: 20.0 }
        s.sum + s.product
    "#,
    )
    .expect_number(30.0);
}

// BUG-14: Object destructuring in match
#[test]
fn bug14_object_destructure_match() {
    ShapeTest::new(
        r#"
        let p = { x: 5, y: 3 }
        match p {
            {x, y} where x > y => x - y,
            _ => 0
        }
    "#,
    )
    .expect_number(2.0);
}

// BUG-15: let in ref fn copies, not aliases
#[test]
fn bug15_let_copies_in_ref_fn() {
    ShapeTest::new(
        r#"
        fn swap(&a, &b) {
            let old = a
            a = b
            b = old
        }
        let mut x = 1
        let mut y = 2
        swap(&x, &y)
        x * 10 + y
    "#,
    )
    .expect_number(21.0);
}

// BUG-16 (WF-3A-tail): a bare module-qualified scalar builtin call used
// DIRECTLY in operand position must infer its declared return type. Before
// the inference-tier module-schema return-type propagation, `time::millis()`
// (declared `-> number`) inferred to `unknown` in operand position, so
// `time::millis() - start` rejected with "operand types are unknown and
// number". The let-binding form already worked (emit-tier stamp); this is the
// bare-operand form. `number - number` -> number.
#[test]
fn bug16_module_call_number_operand_position() {
    ShapeTest::new(
        r#"
        use std::core::time
        let start = time::millis()
        let dt = time::millis() - start
        print(dt >= 0)
    "#,
    )
    .with_stdlib()
    .expect_output_contains("true");
}

// BUG-16b: both operands bare module calls — `millis() - millis()` infers
// number on each side and subtracts to number.
#[test]
fn bug16b_module_call_both_bare_operands() {
    ShapeTest::new(
        r#"
        use std::core::time
        let d = time::millis() - time::millis()
        print(d <= 0)
    "#,
    )
    .with_stdlib()
    .expect_output_contains("true");
}

// BUG-16c: the recovered type is the ACTUAL declared type, not a fabricated
// pass-anything. A `number`-returning module call used where a `string` is
// required (string concatenation) is still a strict compile error.
#[test]
fn bug16c_module_call_number_in_string_context_errors() {
    ShapeTest::new(
        r#"
        use std::core::time
        let x = "elapsed: " + time::millis()
        print(x)
    "#,
    )
    .with_stdlib()
    .expect_run_err_contains("string");
}

// BUG-16d (WF-3A-tail INFERENCE-tier): the emit-tier let-stamp only fixed the
// binop-operand case. A bare module-qualified scalar call in ARGUMENT position
// was inferred by the semantic analyzer (the constraint solver), which held no
// module-export signatures and erased the call to a fresh var — silently
// unifying with the callee's declared param type. `time::millis()` (number)
// passed to a `string` parameter therefore compiled and ran with a heap/number
// mismatch. It must now be a strict compile error: the analyzer recovers the
// declared `number` from the compiler-supplied module-schema map.
#[test]
fn bug16d_module_call_arg_position_wrong_type_errors() {
    ShapeTest::new(
        r#"
        use std::core::time
        fn needs_string(s: string) { print(s) }
        needs_string(time::millis())
    "#,
    )
    .with_stdlib()
    .expect_run_err_contains("string");
}

// BUG-16e: the analyzer must accept a module-qualified `number` call in a
// `number` argument position (positive control — the recovered type is the
// real `number`, not a reject-everything). Proves the fix did not merely start
// rejecting all module calls.
#[test]
fn bug16e_module_call_arg_position_correct_type_accepts() {
    ShapeTest::new(
        r#"
        use std::core::time
        fn takes_number(n: number) { print(n >= 0.0) }
        takes_number(time::millis())
    "#,
    )
    .with_stdlib()
    .expect_output_contains("true");
}

// BUG-16f: strict int/number separation for a module-call operand. `number -
// int` must reject in the analyzer just as `number_var - int_var` does — the
// operand-type recovery must not open a silent int<->number coercion path.
#[test]
fn bug16f_module_call_number_minus_int_errors() {
    ShapeTest::new(
        r#"
        use std::core::time
        let i = 5
        let d = time::millis() - i
        print(d)
    "#,
    )
    .with_stdlib()
    .expect_run_err_contains("int");
}
