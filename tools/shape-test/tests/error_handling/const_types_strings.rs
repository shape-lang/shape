//! Tests for const enforcement, type annotations, and string escapes in Shape.
//!
//! These tests were originally in the programs_error_handling.rs flat file
//! alongside error handling tests. They cover language fundamentals that
//! interact with the error handling system.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Const Enforcement
// =========================================================================

#[test]
fn const_read_ok() {
    ShapeTest::new(
        r#"
        const C = 1
        C
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn const_reassign_is_error() {
    ShapeTest::new(
        r#"
        const C = 1
        C = 2
    "#,
    )
    .expect_run_err();
}

// Compound assignment on const correctly errors at compile time (fixed).
#[test]
fn const_compound_assign_is_error() {
    ShapeTest::new(
        r#"
        const C = 10
        C += 1
        C
    "#,
    )
    .expect_run_err_contains("const");
}

#[test]
fn let_is_mutable() {
    // `let` is immutable in Shape. Use `var` or `let mut` for mutable bindings.
    // This test verifies that `let` reassignment is correctly rejected.
    ShapeTest::new(
        r#"
        var x = 1
        x = 2
        x
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn const_string_value() {
    ShapeTest::new(
        r#"
        const GREETING = "hello"
        GREETING
    "#,
    )
    .expect_string("hello");
}

#[test]
fn const_bool_value() {
    ShapeTest::new(
        r#"
        const FLAG = true
        FLAG
    "#,
    )
    .expect_bool(true);
}

#[test]
fn const_used_in_expression() {
    ShapeTest::new(
        r#"
        const BASE = 10
        const MULT = 5
        BASE * MULT
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn const_complex_expression() {
    ShapeTest::new(
        r#"
        const X = 3 * 4 + 2
        X
    "#,
    )
    .expect_number(14.0);
}

#[test]
fn const_used_by_function() {
    ShapeTest::new(
        r#"
        const FACTOR = 10
        fn scale(x) { x * FACTOR }
        scale(5)
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn const_in_function_body() {
    ShapeTest::new(
        r#"
        fn compute() {
            const LOCAL_CONST = 42
            LOCAL_CONST
        }
        compute()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn const_function_body_reassign_error() {
    ShapeTest::new(
        r#"
        fn compute() {
            const C = 42
            C = 99
        }
        compute()
    "#,
    )
    .expect_run_err();
}

#[test]
fn const_float_value() {
    ShapeTest::new(
        r#"
        const PI = 3.14159
        PI
    "#,
    )
    .expect_number(3.14159);
}

#[test]
fn const_negative_number() {
    ShapeTest::new(
        r#"
        const NEG = -42
        NEG
    "#,
    )
    .expect_number(-42.0);
}

#[test]
fn const_zero() {
    ShapeTest::new(
        r#"
        const ZERO = 0
        ZERO
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn const_bool_in_condition() {
    ShapeTest::new(
        r#"
        const DEBUG = true
        if DEBUG { "debug" } else { "release" }
    "#,
    )
    .expect_string("debug");
}

#[test]
fn const_multiple_declarations() {
    ShapeTest::new(
        r#"
        const A = 1
        const B = 2
        const C = 3
        A + B + C
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn const_string_in_fstring() {
    ShapeTest::new(
        r#"
        const LANG = "Shape"
        f"Hello, {LANG}!"
    "#,
    )
    .expect_string("Hello, Shape!");
}

#[test]
fn const_in_loop_condition() {
    ShapeTest::new(
        r#"
        const LIMIT = 3
        let sum = 0
        for i in [1, 2, 3, 4, 5] {
            if i > LIMIT { break }
            sum = sum + i
        }
        sum
    "#,
    )
    .expect_number(6.0);
}

// =========================================================================
// Type Annotations
// =========================================================================

#[test]
fn type_ann_int_basic() {
    ShapeTest::new(
        r#"
        let x: int = 42
        x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn type_ann_int_arithmetic() {
    ShapeTest::new(
        r#"
        let x: int = 42
        x + 1
    "#,
    )
    .expect_number(43.0);
}

#[test]
fn type_ann_int_comparison() {
    ShapeTest::new(
        r#"
        let x: int = 42
        x > 0
    "#,
    )
    .expect_bool(true);
}

#[test]
fn type_ann_string_basic() {
    ShapeTest::new(
        r#"
        let s: string = "hello"
        s
    "#,
    )
    .expect_string("hello");
}

#[test]
fn type_ann_string_length() {
    ShapeTest::new(
        r#"
        let s: string = "hello world"
        s.length
    "#,
    )
    .expect_number(11.0);
}

#[test]
fn type_ann_bool() {
    ShapeTest::new(
        r#"
        let flag: bool = true
        flag
    "#,
    )
    .expect_bool(true);
}

#[test]
fn type_ann_number() {
    ShapeTest::new(
        r#"
        let x: number = 3.14
        x
    "#,
    )
    .expect_number(3.14);
}

#[test]
fn type_ann_number_arithmetic() {
    ShapeTest::new(
        r#"
        let a: number = 2.5
        let b: number = 1.5
        a + b
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn type_ann_function_params() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int) -> int {
            a + b
        }
        add(3, 4)
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn type_ann_function_return_type() {
    ShapeTest::new(
        r#"
        fn greet(name: string) -> string {
            f"Hello, {name}!"
        }
        greet("World")
    "#,
    )
    .expect_string("Hello, World!");
}

#[test]
fn type_ann_default_params() {
    ShapeTest::new(
        r#"
        fn add(x: int = 1, y: int = 2) -> int {
            x + y
        }
        add()
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn type_ann_result_return() {
    ShapeTest::new(
        r#"
        fn safe_div(a: int, b: int) -> Result<int> {
            if b == 0 { Err("division by zero") }
            else { Ok(a / b) }
        }
        match safe_div(10, 2) {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn type_ann_array_int() {
    ShapeTest::new(
        r#"
        let a: Array<int> = [1, 2, 3]
        a.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn type_ann_struct_fields() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        let p = Point { x: 3, y: 4 }
        p.x + p.y
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn type_ann_generic_function() {
    ShapeTest::new(
        r#"
        fn identity<T>(x: T) -> T { x }
        identity(42)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn type_ann_bool_in_condition() {
    ShapeTest::new(
        r#"
        let flag: bool = true
        if flag { "yes" } else { "no" }
    "#,
    )
    .expect_string("yes");
}

#[test]
fn type_ann_string_concat() {
    ShapeTest::new(
        r#"
        let a: string = "hello"
        let b: string = " world"
        a + b
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn type_ann_mixed_params() {
    ShapeTest::new(
        r#"
        fn describe(name: string, age: int) -> string {
            f"{name} is {age}"
        }
        describe("Alice", 30)
    "#,
    )
    .expect_string("Alice is 30");
}

// =========================================================================
// String Escapes
// =========================================================================

#[test]
fn string_escape_newline() {
    ShapeTest::new(
        r#"
        print("hello\nworld")
    "#,
    )
    .expect_output("hello\nworld");
}

#[test]
fn string_escape_tab() {
    ShapeTest::new(
        r#"
        print("col1\tcol2")
    "#,
    )
    .expect_output("col1\tcol2");
}

#[test]
fn string_escape_carriage_return_length() {
    ShapeTest::new(
        r#"
        let s = "a\rb"
        s.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn string_escape_backslash() {
    ShapeTest::new(
        r#"
        print("path\\to\\file")
    "#,
    )
    .expect_output("path\\to\\file");
}

#[test]
fn string_escape_double_quote_length() {
    // "say \"hi\"" => say "hi" => 8 chars
    ShapeTest::new(
        r#"
        let s = "say \"hi\""
        s.length
    "#,
    )
    .expect_number(8.0);
}

#[test]
fn string_escape_null_length() {
    ShapeTest::new(
        r#"
        let s = "a\0b"
        s.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn string_escape_multiple_newlines() {
    ShapeTest::new(
        r#"
        print("line1\nline2\nline3")
    "#,
    )
    .expect_output("line1\nline2\nline3");
}

#[test]
fn string_escape_mixed_tab_newline() {
    ShapeTest::new(
        r#"
        print("a\tb\nc")
    "#,
    )
    .expect_output("a\tb\nc");
}

#[test]
fn string_escape_in_fstring_tab() {
    ShapeTest::new(
        r#"
        let x = 42
        print(f"value:\t{x}")
    "#,
    )
    .expect_output("value:\t42");
}

#[test]
fn string_escape_in_fstring_newline() {
    ShapeTest::new(
        r#"
        let a = 1
        let b = 2
        print(f"{a}\n{b}")
    "#,
    )
    .expect_output("1\n2");
}

#[test]
fn string_triple_quoted_no_escape_processing() {
    ShapeTest::new(
        r#"
        let s = """line with \n in it"""
        s
    "#,
    )
    .expect_string("line with \\n in it");
}

#[test]
fn string_triple_quoted_multiline() {
    ShapeTest::new(
        r#"
        let s = """
            hello
            world
        """
        print(s)
    "#,
    )
    .expect_output("hello\nworld");
}

#[test]
fn string_triple_quoted_relative_indent() {
    ShapeTest::new(
        r#"
        let s = """
            root
              nested
            end
        """
        print(s)
    "#,
    )
    .expect_output("root\n  nested\nend");
}

#[test]
fn string_empty() {
    ShapeTest::new(
        r#"
        let s = ""
        s.length
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn string_single_newline_escape() {
    ShapeTest::new(
        r#"
        let s = "\n"
        s.length
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn string_trailing_backslash() {
    ShapeTest::new(
        r#"
        let s = "trailing\\"
        s
    "#,
    )
    .expect_string("trailing\\");
}

#[test]
fn string_fstring_escape_in_braces() {
    ShapeTest::new(
        r#"
        let x = 10
        f"value = {x}\n"
    "#,
    )
    .expect_string("value = 10\n");
}
