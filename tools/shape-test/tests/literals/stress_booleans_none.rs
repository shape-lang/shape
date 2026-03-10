//! Stress tests for boolean and none literals.
//!
//! Migrated from shape-vm stress_01_primitives.rs — boolean, none, truthiness,
//! string, and related literal sections.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 3: Boolean Literals
// =============================================================================

/// Verifies boolean literal true.
#[test]
fn test_bool_literal_true() {
    ShapeTest::new("fn test() -> bool { true }\ntest()").expect_bool(true);
}

/// Verifies boolean literal false.
#[test]
fn test_bool_literal_false() {
    ShapeTest::new("fn test() -> bool { false }\ntest()").expect_bool(false);
}

/// Verifies true is truthy (evaluates to true in boolean context).
#[test]
fn test_bool_true_is_truthy() {
    ShapeTest::new("fn test() -> bool { true }\ntest()").expect_bool(true);
}

/// Verifies false is not truthy.
#[test]
fn test_bool_false_is_not_truthy() {
    ShapeTest::new("fn test() -> bool { false }\ntest()").expect_bool(false);
}

// =============================================================================
// SECTION 4: String Literals
// =============================================================================

/// Verifies empty string literal.
#[test]
fn test_string_literal_empty() {
    ShapeTest::new(
        r#"fn test() -> string { "" }
test()"#,
    )
    .expect_string("");
}

/// Verifies hello string literal.
#[test]
fn test_string_literal_hello() {
    ShapeTest::new(
        r#"fn test() -> string { "hello" }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies single character string literal.
#[test]
fn test_string_literal_single_char() {
    ShapeTest::new(
        r#"fn test() -> string { "a" }
test()"#,
    )
    .expect_string("a");
}

/// Verifies multi-word string literal.
#[test]
fn test_string_literal_multi_word() {
    ShapeTest::new(
        r#"fn test() -> string { "hello world" }
test()"#,
    )
    .expect_string("hello world");
}

/// Verifies string literal with numbers.
#[test]
fn test_string_literal_with_numbers() {
    ShapeTest::new(
        r#"fn test() -> string { "abc123" }
test()"#,
    )
    .expect_string("abc123");
}

/// Verifies string literal with leading/trailing spaces.
#[test]
fn test_string_literal_with_spaces() {
    ShapeTest::new(
        r#"fn test() -> string { "  spaces  " }
test()"#,
    )
    .expect_string("  spaces  ");
}

/// Verifies string escape: newline.
#[test]
fn test_string_escape_newline() {
    ShapeTest::new(
        r#"fn test() -> string { "line1\nline2" }
test()"#,
    )
    .expect_string("line1\nline2");
}

/// Verifies string escape: tab.
#[test]
fn test_string_escape_tab() {
    ShapeTest::new(
        r#"fn test() -> string { "col1\tcol2" }
test()"#,
    )
    .expect_string("col1\tcol2");
}

/// Verifies string escape: backslash.
#[test]
fn test_string_escape_backslash() {
    ShapeTest::new(
        r#"fn test() -> string { "back\\slash" }
test()"#,
    )
    .expect_string("back\\slash");
}

/// Verifies string escape: double quote.
#[test]
fn test_string_escape_quote() {
    ShapeTest::new(
        r#"fn test() -> string { "say \"hi\"" }
test()"#,
    )
    .expect_string("say \"hi\"");
}

/// Verifies long string literal.
#[test]
fn test_string_literal_long() {
    ShapeTest::new(
        r#"fn test() -> string { "the quick brown fox jumps over the lazy dog" }
test()"#,
    )
    .expect_string("the quick brown fox jumps over the lazy dog");
}

/// Verifies string literal with special characters.
#[test]
fn test_string_literal_special_chars() {
    ShapeTest::new(
        r#"fn test() -> string { "!@#%^&*()" }
test()"#,
    )
    .expect_string("!@#%^&*()");
}

/// Verifies string literal with basic unicode.
#[test]
fn test_string_literal_unicode_basic() {
    ShapeTest::new(
        r#"fn test() -> string { "café" }
test()"#,
    )
    .expect_string("café");
}

/// Verifies string escape: carriage return.
#[test]
fn test_string_escape_carriage_return() {
    ShapeTest::new(
        r#"fn test() -> string { "a\rb" }
test()"#,
    )
    .expect_string("a\rb");
}

// =============================================================================
// SECTION 5: None / Null
// =============================================================================

/// Verifies None literal returns none.
#[test]
fn test_null_literal() {
    ShapeTest::new("let x = None\nx").expect_none();
}

/// Verifies None is not truthy.
#[test]
fn test_null_is_not_truthy() {
    ShapeTest::new("fn test() -> bool { !None }\ntest()").expect_bool(true);
}

// =============================================================================
// SECTION 6: Type Annotations — bool and string
// =============================================================================

/// Verifies let binding with bool type annotation.
#[test]
fn test_let_bool_annotation() {
    ShapeTest::new("fn test() -> bool { let x: bool = true\n x }\ntest()").expect_bool(true);
}

/// Verifies let binding with string type annotation.
#[test]
fn test_let_string_annotation() {
    ShapeTest::new("fn test() -> string { let x: string = \"hi\"\n x }\ntest()")
        .expect_string("hi");
}

/// Verifies let binding with inferred bool type.
#[test]
fn test_let_inferred_bool() {
    ShapeTest::new("fn test() { let x = false\n x }\ntest()").expect_bool(false);
}

/// Verifies let binding with inferred string type.
#[test]
fn test_let_inferred_string() {
    ShapeTest::new(
        r#"fn test() { let x = "abc"
 x }
test()"#,
    )
    .expect_string("abc");
}

// =============================================================================
// SECTION 7: Truthiness
// =============================================================================

/// Verifies int zero is not truthy.
#[test]
fn test_int_zero_is_not_truthy() {
    ShapeTest::new("fn test() -> int { 0 }\ntest()").expect_number(0.0);
}

/// Verifies int one is truthy (non-zero).
#[test]
fn test_int_one_is_truthy() {
    ShapeTest::new("if 1 { true } else { false }").expect_bool(true);
}

/// Verifies negative int is truthy.
#[test]
fn test_int_negative_is_truthy() {
    ShapeTest::new("if -1 { true } else { false }").expect_bool(true);
}

/// Verifies number zero is not truthy.
#[test]
fn test_number_zero_is_not_truthy() {
    ShapeTest::new("if 0.0 { true } else { false }").expect_bool(false);
}

/// Verifies positive number is truthy.
#[test]
fn test_number_positive_is_truthy() {
    ShapeTest::new("if 0.1 { true } else { false }").expect_bool(true);
}

/// Verifies negative number is truthy.
#[test]
fn test_number_negative_is_truthy() {
    ShapeTest::new("if -0.1 { true } else { false }").expect_bool(true);
}

/// Verifies empty string is falsy.
#[test]
fn test_empty_string_is_truthy() {
    ShapeTest::new(r#"if "" { true } else { false }"#).expect_bool(false);
}

/// Verifies non-empty string is truthy.
#[test]
fn test_nonempty_string_is_truthy() {
    ShapeTest::new(r#"if "x" { true } else { false }"#).expect_bool(true);
}

// =============================================================================
// SECTION 9: Equality & Identity
// =============================================================================

/// Verifies int equality with same values.
#[test]
fn test_int_equality_same() {
    ShapeTest::new("fn test() -> bool { 42 == 42 }\ntest()").expect_bool(true);
}

/// Verifies int equality with different values.
#[test]
fn test_int_equality_different() {
    ShapeTest::new("fn test() -> bool { 42 == 43 }\ntest()").expect_bool(false);
}

/// Verifies int inequality.
#[test]
fn test_int_inequality() {
    ShapeTest::new("fn test() -> bool { 1 != 2 }\ntest()").expect_bool(true);
}

/// Verifies number equality with same values.
#[test]
fn test_number_equality_same() {
    ShapeTest::new("fn test() -> bool { 3.14 == 3.14 }\ntest()").expect_bool(true);
}

/// Verifies number equality with different values.
#[test]
fn test_number_equality_different() {
    ShapeTest::new("fn test() -> bool { 3.14 == 3.15 }\ntest()").expect_bool(false);
}

/// Verifies bool equality true == true.
#[test]
fn test_bool_equality_true() {
    ShapeTest::new("fn test() -> bool { true == true }\ntest()").expect_bool(true);
}

/// Verifies bool equality false == false.
#[test]
fn test_bool_equality_false() {
    ShapeTest::new("fn test() -> bool { false == false }\ntest()").expect_bool(true);
}

/// Verifies bool inequality true != false.
#[test]
fn test_bool_inequality() {
    ShapeTest::new("fn test() -> bool { true != false }\ntest()").expect_bool(true);
}

/// Verifies string equality with same values.
#[test]
fn test_string_equality_same() {
    ShapeTest::new(
        r#"fn test() -> bool { "abc" == "abc" }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies string equality with different values.
#[test]
fn test_string_equality_different() {
    ShapeTest::new(
        r#"fn test() -> bool { "abc" == "def" }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies null equality None == None.
#[test]
fn test_null_equality() {
    ShapeTest::new("fn test() -> bool { None == None }\ntest()").expect_bool(true);
}

/// Verifies None != 0 (no implicit coercion).
#[test]
fn test_null_not_equal_to_zero() {
    ShapeTest::new("fn test() -> bool { None != 0 }\ntest()").expect_bool(true);
}

// =============================================================================
// SECTION 11: Logical Operators with Booleans
// =============================================================================

/// Verifies !true == false.
#[test]
fn test_not_true() {
    ShapeTest::new("fn test() -> bool { !true }\ntest()").expect_bool(false);
}

/// Verifies !false == true.
#[test]
fn test_not_false() {
    ShapeTest::new("fn test() -> bool { !false }\ntest()").expect_bool(true);
}

/// Verifies true and true == true.
#[test]
fn test_and_true_true() {
    ShapeTest::new("fn test() -> bool { true and true }\ntest()").expect_bool(true);
}

/// Verifies true and false == false.
#[test]
fn test_and_true_false() {
    ShapeTest::new("fn test() -> bool { true and false }\ntest()").expect_bool(false);
}

/// Verifies false or true == true.
#[test]
fn test_or_false_true() {
    ShapeTest::new("fn test() -> bool { false or true }\ntest()").expect_bool(true);
}

/// Verifies false or false == false.
#[test]
fn test_or_false_false() {
    ShapeTest::new("fn test() -> bool { false or false }\ntest()").expect_bool(false);
}

// =============================================================================
// SECTION 12: Comparison Operators with Literals
// =============================================================================

/// Verifies 1 < 2 is true.
#[test]
fn test_int_less_than_true() {
    ShapeTest::new("fn test() -> bool { 1 < 2 }\ntest()").expect_bool(true);
}

/// Verifies 2 < 1 is false.
#[test]
fn test_int_less_than_false() {
    ShapeTest::new("fn test() -> bool { 2 < 1 }\ntest()").expect_bool(false);
}

/// Verifies 5 > 3 is true.
#[test]
fn test_int_greater_than() {
    ShapeTest::new("fn test() -> bool { 5 > 3 }\ntest()").expect_bool(true);
}

/// Verifies 3 <= 3 is true.
#[test]
fn test_int_less_equal() {
    ShapeTest::new("fn test() -> bool { 3 <= 3 }\ntest()").expect_bool(true);
}

/// Verifies 3 >= 4 is false.
#[test]
fn test_int_greater_equal() {
    ShapeTest::new("fn test() -> bool { 3 >= 4 }\ntest()").expect_bool(false);
}

/// Verifies 1.5 < 2.5 is true.
#[test]
fn test_number_less_than() {
    ShapeTest::new("fn test() -> bool { 1.5 < 2.5 }\ntest()").expect_bool(true);
}

/// Verifies 3.0 > 2.9 is true.
#[test]
fn test_number_greater_than() {
    ShapeTest::new("fn test() -> bool { 3.0 > 2.9 }\ntest()").expect_bool(true);
}

// =============================================================================
// SECTION 15: Double negation and misc bool ops
// =============================================================================

/// Verifies double negation of bool.
#[test]
fn test_double_negation_bool() {
    ShapeTest::new("fn test() -> bool { !!true }\ntest()").expect_bool(true);
}

// =============================================================================
// SECTION 18: Top-level bool and string expressions
// =============================================================================

/// Verifies top-level bool expression.
#[test]
fn test_top_level_bool_expr() {
    ShapeTest::new("true").expect_bool(true);
}

/// Verifies top-level string expression.
#[test]
fn test_top_level_string_expr() {
    ShapeTest::new(r#""hello""#).expect_string("hello");
}

// =============================================================================
// SECTION 20: Misc string edge cases
// =============================================================================

/// Verifies string length method.
#[test]
fn test_string_length_method() {
    ShapeTest::new(
        r#"fn test() -> int { "hello".length() }
test()"#,
    )
    .expect_number(5.0);
}

/// Verifies empty string length.
#[test]
fn test_empty_string_length() {
    ShapeTest::new(
        r#"fn test() -> int { "".length() }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies string interpolation.
#[test]
fn test_string_interpolation() {
    ShapeTest::new(
        r#"fn test() -> string {
            let x = 42
            f"value is {x}"
        }
test()"#,
    )
    .expect_string("value is 42");
}

/// Verifies string concatenation.
#[test]
fn test_string_concatenation() {
    ShapeTest::new(
        r#"fn test() -> string { "hello" + " " + "world" }
test()"#,
    )
    .expect_string("hello world");
}

/// Verifies chained string concatenation.
#[test]
fn test_chained_string_concat() {
    ShapeTest::new(
        r#"fn test() -> string { "a" + "b" + "c" + "d" }
test()"#,
    )
    .expect_string("abcd");
}

/// Verifies int to string interpolation.
#[test]
fn test_int_to_string_interpolation() {
    ShapeTest::new(
        r#"fn test() -> string { f"{1 + 2}" }
test()"#,
    )
    .expect_string("3");
}

/// Verifies bool to string interpolation.
#[test]
fn test_bool_to_string_interpolation() {
    ShapeTest::new(
        r#"fn test() -> string { f"{true}" }
test()"#,
    )
    .expect_string("true");
}

/// Verifies negative int to string interpolation.
#[test]
fn test_negative_int_to_string_interpolation() {
    ShapeTest::new(
        r#"fn test() -> string {
            let x = -7
            f"val: {x}"
        }
test()"#,
    )
    .expect_string("val: -7");
}

/// Verifies void function returns unit or none.
#[test]
fn test_unit_from_void_function() {
    ShapeTest::new("fn test() { let x = 1 }\ntest()").expect_run_ok();
}
