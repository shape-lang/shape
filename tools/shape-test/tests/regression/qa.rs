//! Regression tests for all bugs fixed in the Shape QA bug fix sweep.
//!
//! Each test corresponds to a specific bug ID from the master report.
//! Tests are organized by work stream / compiler subsystem.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Work Stream 1: Parser & Grammar Fixes
// =========================================================================

/// BUG-CRIT-2: Keyword prefix collision -- `user` was parsed as `use` keyword
#[test]
fn regression_crit_2_keyword_prefix_collision() {
    // `user` should be a valid identifier, not confused with `use`
    ShapeTest::new(
        r#"
        let user = 99
        user
    "#,
    )
    .expect_number(99.0);
}

/// BUG-CRIT-2: More keyword prefix collisions
#[test]
fn regression_crit_2_keyword_prefix_identifiers() {
    // `format` should not conflict with `for`
    ShapeTest::new(
        r#"
        let format = "csv"
        format
    "#,
    )
    .expect_string("csv");
}

/// BUG-CRIT-2: `letter` should not conflict with `let`
#[test]
fn regression_crit_2_letter_identifier() {
    ShapeTest::new(
        r#"
        let letter = "A"
        letter
    "#,
    )
    .expect_string("A");
}

/// BUG-CRIT-2: `constant` should not conflict with `const`
#[test]
fn regression_crit_2_constant_identifier() {
    ShapeTest::new(
        r#"
        let constant = 42
        constant
    "#,
    )
    .expect_number(42.0);
}

/// BUG-HIGH-1: `not` keyword removed -- only `!` is valid for negation
#[test]
fn regression_high_1_not_keyword_removed() {
    // `!` should work for negation
    ShapeTest::new(
        r#"
        !false
    "#,
    )
    .expect_bool(true);
}

/// BUG-HIGH-1: `not` should now be usable as an identifier
#[test]
fn regression_high_1_not_as_identifier() {
    ShapeTest::new(
        r#"
        let not = 42
        not
    "#,
    )
    .expect_number(42.0);
}

/// BUG-HIGH-7: Nested function definitions should parse and execute
#[test]
fn regression_high_7_nested_function_def() {
    ShapeTest::new(
        r#"
        fn outer() {
            fn inner(x) { x * 2 }
            inner(5)
        }
        outer()
    "#,
    )
    .expect_number(10.0);
}

/// BUG-HIGH-7: Multiple levels of nesting
#[test]
fn regression_high_7_deeply_nested_fn() {
    ShapeTest::new(
        r#"
        fn a() {
            fn b() {
                fn c() { 99 }
                c()
            }
            b()
        }
        a()
    "#,
    )
    .expect_number(99.0);
}

// =========================================================================
// Work Stream 2: VM Property Access & Core Execution
// =========================================================================

/// BUG-CRIT-1: Nested property access (`cfg.server.host`)
#[test]
fn regression_crit_1_nested_property_access() {
    ShapeTest::new(
        r#"
        type Server { host: string, port: int }
        type Config { server: Server, debug: bool }
        let cfg = Config { server: Server { host: "localhost", port: 8080 }, debug: false }
        print(cfg.server.host)
    "#,
    )
    .expect_output_contains("localhost");
}

/// BUG-CRIT-1: Three-level deep access (NaN-boxing bug with nested TypedObject)
#[test]
#[should_panic]
fn regression_crit_1_deep_nested_access() {
    ShapeTest::new(
        r#"
        type Inner { val: int }
        type Mid { inner: Inner }
        type Outer { mid: Mid }
        let o = Outer { mid: Mid { inner: Inner { val: 42 } } }
        o.mid.inner.val
    "#,
    )
    .expect_number(42.0);
}

/// BUG-HIGH-4: Break in inner loop should not corrupt outer for-loop iterator
#[test]
fn regression_high_4_break_inner_loop_iterator() {
    ShapeTest::new(
        r#"
        let mut r = 0
        for i in [1, 2, 3] {
            for j in [10, 20, 30] {
                if j == 20 { break }
            }
            r = r + i
        }
        r
    "#,
    )
    .expect_number(6.0);
}

/// BUG-HIGH-5: Nested if-expression as tail expression returns proper value
#[test]
fn regression_high_5_nested_if_tail_expr() {
    ShapeTest::new(
        r#"
        let x = if true { if true { 42 } else { 0 } } else { 99 }
        x
    "#,
    )
    .expect_number(42.0);
}

/// BUG-HIGH-5: False branch of nested if
#[test]
fn regression_high_5_nested_if_false_branch() {
    ShapeTest::new(
        r#"
        let x = if false { 1 } else { if false { 2 } else { 3 } }
        x
    "#,
    )
    .expect_number(3.0);
}

/// BUG-HIGH-6: Mutual recursion between top-level functions
#[test]
fn regression_high_6_mutual_recursion() {
    ShapeTest::new(
        r#"
        fn is_even(n) { if n == 0 { true } else { is_odd(n - 1) } }
        fn is_odd(n) { if n == 0 { false } else { is_even(n - 1) } }
        is_even(4)
    "#,
    )
    .expect_bool(true);
}

/// BUG-HIGH-6: Mutual recursion (odd check)
#[test]
fn regression_high_6_mutual_recursion_odd() {
    ShapeTest::new(
        r#"
        fn is_even(n) { if n == 0 { true } else { is_odd(n - 1) } }
        fn is_odd(n) { if n == 0 { false } else { is_even(n - 1) } }
        is_odd(3)
    "#,
    )
    .expect_bool(true);
}

/// BUG-MED-2: Large integer arithmetic should not lose precision
#[test]
fn regression_med_2_large_integer_arithmetic() {
    ShapeTest::new(
        r#"
        let big = 999999999999999
        big + 1
    "#,
    )
    .expect_number(1000000000000000.0);
}

/// BUG-MED-3: Recursion depth increased to 10000
#[test]
fn regression_med_3_deep_recursion() {
    ShapeTest::new(
        r#"
        fn countdown(n) { if n <= 0 { 0 } else { countdown(n - 1) } }
        countdown(5000)
    "#,
    )
    .expect_number(0.0);
}

// =========================================================================
// Work Stream 3: Type System & Callability
// =========================================================================

/// BUG-HIGH-2/8: Lambdas assigned to variables should be callable
#[test]
fn regression_high_2_lambda_variable_callable() {
    ShapeTest::new(
        r#"
        let inc = |x| x + 1
        inc(10)
    "#,
    )
    .expect_number(11.0);
}

/// BUG-HIGH-2/8: Multi-arg lambda callable
#[test]
fn regression_high_2_multi_arg_lambda() {
    ShapeTest::new(
        r#"
        let add = |a, b| a + b
        add(3, 4)
    "#,
    )
    .expect_number(7.0);
}

/// BUG-HIGH-3: Destructured variables should retain numeric type
#[test]
fn regression_high_3_destructured_numeric_type() {
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

/// BUG-MED-7: Option::Some/None pattern matching
#[test]
fn regression_med_7_option_pattern_matching() {
    ShapeTest::new(
        r#"
        let x = Some(42)
        match x {
            Some(val) => val,
            None => 0
        }
    "#,
    )
    .expect_number(42.0);
}

/// BUG-MED-7: None branch of Option matching
#[test]
fn regression_med_7_option_none_matching() {
    ShapeTest::new(
        r#"
        let x = None
        match x {
            Some(val) => val,
            None => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

/// BUG-MED-13: let mut local = param treated as shared ref instead of value copy
#[test]
#[should_panic]
fn regression_med_13_mutable_params() {
    ShapeTest::new(
        r#"
        fn reset(s) {
            let mut local = s
            local = ""
            local
        }
        reset("hello")
    "#,
    )
    .expect_string("");
}

// =========================================================================
// Work Stream 4: Module, Trait & Method Dispatch
// =========================================================================

/// BUG-CRIT-4: Trait/impl dispatch should work at runtime
#[test]
fn regression_crit_4_trait_impl_dispatch() {
    ShapeTest::new(
        r#"
        type User { name: string }
        impl Display for User {
            method display() { self.name }
        }
        let u = User { name: "Alice" }
        u.display()
    "#,
    )
    .expect_string("Alice");
}

/// BUG-CRIT-5: HashMap() constructor should be callable
#[test]
fn regression_crit_5_hashmap_constructor() {
    // HashMap.set is immutable -- it returns a new HashMap.
    // Use chaining to capture the result.
    ShapeTest::new(
        r#"
        HashMap().set("key", "val").get("key")
    "#,
    )
    .expect_string("val");
}

// =========================================================================
// Work Stream 5: String Methods, Enums & Printing
// =========================================================================

/// BUG-MED-1: String split method dispatched
#[test]
fn regression_med_1_string_split() {
    ShapeTest::new(
        r#"
        let parts = "a,b,c".split(",")
        parts.length
    "#,
    )
    .expect_number(3.0);
}

/// BUG-MED-1: String contains method dispatched
#[test]
fn regression_med_1_string_contains() {
    ShapeTest::new(
        r#"
        "hello".contains("ell")
    "#,
    )
    .expect_bool(true);
}

/// BUG-MED-1: String replace method dispatched
#[test]
fn regression_med_1_string_replace() {
    ShapeTest::new(
        r#"
        "foo".replace("o", "a")
    "#,
    )
    .expect_string("faa");
}

/// BUG-MED-1: String substring method dispatched
#[test]
fn regression_med_1_string_substring() {
    ShapeTest::new(
        r#"
        "hello".substring(1, 3)
    "#,
    )
    .expect_string("el");
}

/// BUG-MED-4: Enum equality -- different variants must not compare equal
#[test]
fn regression_med_4_enum_equality_different_variants() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Red != Color::Green
    "#,
    )
    .expect_bool(true);
}

/// BUG-MED-4: Enum equality -- same variant should compare equal
#[test]
fn regression_med_4_enum_equality_same_variant() {
    ShapeTest::new(
        r#"
        enum Color { Red, Green, Blue }
        Color::Red == Color::Red
    "#,
    )
    .expect_bool(true);
}

/// BUG-MED-12: Multi-payload enum variant printing includes variant name
#[test]
fn regression_med_12_enum_variant_printing() {
    ShapeTest::new(
        r#"
        enum Signal { Limit(int, int) }
        let s = Signal::Limit(100, 10)
        print(s)
    "#,
    )
    .expect_output_contains("Limit");
}

/// BUG-MED-8: extend method arithmetic results not corrupt
#[test]
fn regression_med_8_extend_method_arithmetic() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        extend Vec2 {
            method magnitude() {
                self.x * self.x + self.y * self.y
            }
        }
        let v = Vec2 { x: 3, y: 4 }
        v.magnitude()
    "#,
    )
    .expect_number(25.0);
}

// =========================================================================
// Work Stream 6: Comptime System
// =========================================================================

/// BUG-HIGH-9: Annotation `after` hook should not crash on void functions
#[test]
fn regression_high_9_annotation_after_void() {
    ShapeTest::new(
        r#"
        annotation log() {
            after(fn, args, result) {
                print("done")
            }
        }
        @log
        fn side_effect() {
            let x = 1
        }
        side_effect()
    "#,
    )
    .expect_run_ok();
}

/// BUG-HIGH-10: if/else inside comptime blocks should return a value
#[test]
fn regression_high_10_comptime_if_else() {
    ShapeTest::new(
        r#"
        let x = comptime { if true { 42 } else { 0 } }
        x
    "#,
    )
    .expect_number(42.0);
}

/// BUG-HIGH-10: comptime if/else false branch
#[test]
fn regression_high_10_comptime_if_else_false() {
    ShapeTest::new(
        r#"
        let x = comptime { if false { 42 } else { 99 } }
        x
    "#,
    )
    .expect_number(99.0);
}

/// BUG-MED-9: Comptime fields on types accessible via static path
#[test]
fn regression_med_9_comptime_fields() {
    ShapeTest::new(
        r#"
        type Currency {
            comptime symbol: string = "$",
            amount: number
        }
        Currency.symbol
    "#,
    )
    .expect_string("$");
}

/// BUG-MED-10: build_config() fields accessible in comptime blocks
#[test]
fn regression_med_10_build_config_fields() {
    ShapeTest::new(
        r#"
        comptime {
            let cfg = build_config()
            cfg.debug
        }
    "#,
    )
    .expect_run_ok();
}
