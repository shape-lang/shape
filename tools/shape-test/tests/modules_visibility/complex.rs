//! Complex module programs — modules combined with independent expressions,
//! control flow, closures, and module parse error cases.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// COMPLEX PROGRAMS — Independent of Module Access (~15 tests)
// These test programs that use modules for parsing but evaluate
// independent expressions, or test error cases.
// =============================================================================

#[test]
fn test_complex_modules_then_independent_arithmetic() {
    ShapeTest::new(
        r#"
        mod a { fn val() { 1 } }
        mod b { fn val() { 2 } }
        10 + 20 + 30
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn test_complex_module_then_string_concat() {
    ShapeTest::new(
        r#"
        mod greeter { fn hello() { "hi" } }
        "hello" + " " + "world"
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn test_complex_module_then_boolean_logic() {
    ShapeTest::new(
        r#"
        mod check { fn is_valid(x) { x > 0 } }
        true && !false
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_complex_module_then_if_expression() {
    ShapeTest::new(
        r#"
        mod config { const DEBUG = true }
        if true { 42 } else { 0 }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_complex_module_then_function_def_and_call() {
    ShapeTest::new(
        r#"
        mod unused { fn helper() { 0 } }
        fn double(x) { x * 2 }
        double(21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_complex_module_then_closure() {
    ShapeTest::new(
        r#"
        mod M { fn f() { 0 } }
        let inc = |x| x + 1
        inc(41)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_complex_module_then_for_loop() {
    ShapeTest::new(
        r#"
        mod M { fn f() { 0 } }
        let total = 0
        for x in [1, 2, 3, 4, 5] {
            total = total + x
        }
        total
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_complex_module_then_while_loop() {
    ShapeTest::new(
        r#"
        mod M { fn f() { 0 } }
        let i = 0
        while i < 10 {
            i = i + 1
        }
        i
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_complex_module_then_match() {
    ShapeTest::new(
        r#"
        mod M { fn f() { 0 } }
        let x = "hello"
        match x {
            "hello" => 1,
            "world" => 2,
            _ => 0,
        }
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn test_complex_module_then_nested_functions() {
    ShapeTest::new(
        r#"
        mod M { fn unused() { 0 } }
        fn outer() {
            fn inner(x) { x * 2 }
            inner(5)
        }
        outer()
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_complex_module_then_recursion() {
    ShapeTest::new(
        r#"
        mod M { fn unused() { 0 } }
        fn factorial(n) {
            if n <= 1 { 1 }
            else { n * factorial(n - 1) }
        }
        factorial(5)
    "#,
    )
    .expect_number(120.0);
}

#[test]
fn test_complex_module_then_print_output() {
    ShapeTest::new(
        r#"
        mod M { fn unused() { 0 } }
        print("hello")
        print("world")
    "#,
    )
    .expect_output("hello\nworld");
}

#[test]
fn test_complex_module_then_array_ops() {
    ShapeTest::new(
        r#"
        mod M { fn unused() { 0 } }
        let arr = [10, 20, 30]
        arr[1]
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_complex_module_then_type_construction() {
    ShapeTest::new(
        r#"
        mod M { fn unused() { 0 } }
        type Point { x: number, y: number }
        let p = Point { x: 3, y: 4 }
        p.x + p.y
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn test_complex_module_then_enum_usage() {
    ShapeTest::new(
        r#"
        mod M { fn unused() { 0 } }
        enum Color { Red, Green, Blue }
        Color::Red == Color::Red
    "#,
    )
    .expect_bool(true);
}

// =============================================================================
// MODULE PARSE ERROR CASES (~5 tests)
// =============================================================================

#[test]
fn test_mod_missing_braces() {
    // "mod Broken" without braces — parser may or may not error
    let result = shape_ast::parse_program("mod Broken");
    // If it parses, it should not be a Module item
    if let Ok(program) = result {
        let has_module = program
            .items
            .iter()
            .any(|item| matches!(item, shape_ast::ast::Item::Module(_, _)));
        assert!(
            !has_module,
            "mod without braces should not produce a Module item"
        );
    }
    // If it errors, that's also acceptable
}

#[test]
fn test_mod_missing_name() {
    // "mod { ... }" without a name — grammar requires an identifier
    let result = shape_ast::parse_program("mod { fn f() { } }");
    if let Ok(program) = result {
        let has_module = program
            .items
            .iter()
            .any(|item| matches!(item, shape_ast::ast::Item::Module(_, _)));
        assert!(
            !has_module,
            "mod without name should not produce a Module item"
        );
    }
}

#[test]
fn test_mod_keyword_as_name_rejected() {
    // Using a keyword as module name should fail
    ShapeTest::new("mod if { }").expect_parse_err();
}

#[test]
fn test_mod_underscore_name_parses() {
    ShapeTest::new("mod my_module { }").expect_parse_ok();
}

#[test]
fn test_mod_capitalized_name_parses() {
    ShapeTest::new("mod MyModule { fn create() { 1 } }").expect_parse_ok();
}
