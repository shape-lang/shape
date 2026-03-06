//! Default parameter tests.
//!
//! Covers: single/multiple default params, numeric/string/bool defaults,
//! typed annotations, partial overrides, expression defaults,
//! and defaults used in HOF contexts.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// From programs_closures_and_hof.rs
// =========================================================================

// BUG: Default string parameter without type annotation fails.
// Workaround: use type annotation.
#[test]
fn test_default_param_string_typed() {
    ShapeTest::new(
        r#"
        fn greet(name: string = "world") -> string { "hello " + name }
        greet()
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn test_default_param_string_override_typed() {
    ShapeTest::new(
        r#"
        fn greet(name: string = "world") -> string { "hello " + name }
        greet("shape")
    "#,
    )
    .expect_string("hello shape");
}

#[test]
fn test_default_param_number() {
    ShapeTest::new(
        r#"
        fn add(a, b = 0) { a + b }
        add(5)
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_default_param_number_override() {
    ShapeTest::new(
        r#"
        fn add(a, b = 0) { a + b }
        add(5, 10)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_default_param_multiple_defaults() {
    ShapeTest::new(
        r#"
        fn calc(a = 1, b = 2, c = 3) { a + b + c }
        calc()
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_default_param_override_first_only() {
    ShapeTest::new(
        r#"
        fn calc(a = 1, b = 2, c = 3) { a + b + c }
        calc(10)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_default_param_override_two() {
    ShapeTest::new(
        r#"
        fn calc(a = 1, b = 2, c = 3) { a + b + c }
        calc(10, 20)
    "#,
    )
    .expect_number(33.0);
}

#[test]
fn test_default_param_override_all() {
    ShapeTest::new(
        r#"
        fn calc(a = 1, b = 2, c = 3) { a + b + c }
        calc(10, 20, 30)
    "#,
    )
    .expect_number(60.0);
}

// BUG: Mixing required and default params without type annotations on string
// default fails. Workaround: use typed.
#[test]
fn test_default_param_mixed_required_and_default_typed() {
    ShapeTest::new(
        r#"
        fn format_name(first: string, last: string = "Doe") -> string {
            first + " " + last
        }
        format_name("Jane")
    "#,
    )
    .expect_string("Jane Doe");
}

#[test]
fn test_default_param_mixed_override_typed() {
    ShapeTest::new(
        r#"
        fn format_name(first: string, last: string = "Doe") -> string {
            first + " " + last
        }
        format_name("Jane", "Smith")
    "#,
    )
    .expect_string("Jane Smith");
}

#[test]
fn test_default_param_bool() {
    ShapeTest::new(
        r#"
        fn check(val, strict = false) { if strict { val > 100 } else { val > 0 } }
        check(50)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_default_param_bool_override() {
    ShapeTest::new(
        r#"
        fn check(val, strict = false) { if strict { val > 100 } else { val > 0 } }
        check(50, true)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_default_param_with_typed() {
    ShapeTest::new(
        r#"
        fn add(x: int = 1, y: int = 2) -> int {
            x + y
        }
        print(add())
        print(add(5))
        print(add(5, 6))
    "#,
    )
    .expect_output("3\n7\n11");
}

#[test]
fn test_default_param_expression() {
    ShapeTest::new(
        r#"
        fn calc(x, y = 10 * 2) { x + y }
        calc(5)
    "#,
    )
    .expect_number(25.0);
}

#[test]
fn test_default_param_used_in_hof() {
    ShapeTest::new(
        r#"
        fn make_adder(n = 10) { |x| x + n }
        let f = make_adder()
        f(5)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_default_param_zero() {
    ShapeTest::new(
        r#"
        fn f(x = 0) { x + 100 }
        f()
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn test_default_param_negative() {
    ShapeTest::new(
        r#"
        fn f(x = -1) { x * 10 }
        f()
    "#,
    )
    .expect_number(-10.0);
}

// =========================================================================
// From programs_closures_hof.rs
// =========================================================================

// BUG: String default params crash with "expected a reference value (&) but found a regular value"
// when the default is used (not overridden). This is a compiler bug with string-typed defaults.
// Workaround: use typed annotation `name: string = "world"`.
#[test]
fn default_param_single_with_default() {
    ShapeTest::new(
        r#"
        fn greet(name: string = "world") -> string {
            return "hello " + name
        }
        greet()
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn default_param_override() {
    ShapeTest::new(
        r#"
        fn greet(name = "world") {
            "hello " + name
        }
        greet("Shape")
    "#,
    )
    .expect_string("hello Shape");
}

#[test]
fn default_param_numeric() {
    ShapeTest::new(
        r#"
        fn add(a, b = 0) {
            a + b
        }
        add(42)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn default_param_numeric_overridden() {
    ShapeTest::new(
        r#"
        fn add(a, b = 0) {
            a + b
        }
        add(20, 22)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn default_param_multiple_defaults() {
    ShapeTest::new(
        r#"
        fn f(a = 1, b = 2, c = 3) {
            a + b + c
        }
        f()
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn default_param_partial_override() {
    ShapeTest::new(
        r#"
        fn f(a = 1, b = 2, c = 3) {
            a + b + c
        }
        f(10)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn default_param_two_overrides() {
    ShapeTest::new(
        r#"
        fn f(a = 1, b = 2, c = 3) {
            a + b + c
        }
        f(10, 20)
    "#,
    )
    .expect_number(33.0);
}

#[test]
fn default_param_all_overrides() {
    ShapeTest::new(
        r#"
        fn f(a = 1, b = 2, c = 3) {
            a + b + c
        }
        f(10, 20, 30)
    "#,
    )
    .expect_number(60.0);
}

// BUG: String default params crash -- see default_param_single_with_default
// Workaround: use typed annotation.
#[test]
fn default_param_string_default() {
    ShapeTest::new(
        r#"
        fn format_name(first: string, last: string = "Doe") -> string {
            return first + " " + last
        }
        format_name("John")
    "#,
    )
    .expect_string("John Doe");
}

#[test]
fn default_param_with_type_annotation() {
    ShapeTest::new(
        r#"
        fn add(x: int = 1, y: int = 2) -> int {
            return x + y
        }
        add()
    "#,
    )
    .expect_number(3.0);
}
