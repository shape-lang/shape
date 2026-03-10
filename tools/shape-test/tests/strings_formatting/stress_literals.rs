//! Stress tests for string literals, concatenation, escapes, equality, comparison,
//! variable assignment, conditionals, and function parameter/return with strings.

use shape_test::shape_test::ShapeTest;

// ========================================================================
// 1. String Literal Creation
// ========================================================================

/// Verifies that an empty string literal is created correctly.
#[test]
fn test_string_literal_empty() {
    ShapeTest::new(r#"fn test() -> string { "" } test()"#).expect_string("");
}

/// Verifies a single-character string literal.
#[test]
fn test_string_literal_single_char() {
    ShapeTest::new(r#"fn test() -> string { "a" } test()"#).expect_string("a");
}

/// Verifies a multi-word string literal.
#[test]
fn test_string_literal_multi_word() {
    ShapeTest::new(r#"fn test() -> string { "hello world" } test()"#).expect_string("hello world");
}

/// Verifies a string literal containing digits.
#[test]
fn test_string_literal_with_digits() {
    ShapeTest::new(r#"fn test() -> string { "abc123" } test()"#).expect_string("abc123");
}

/// Verifies a string literal containing special characters.
#[test]
fn test_string_literal_special_chars() {
    ShapeTest::new(r#"fn test() -> string { "!@#$%^&*()" } test()"#).expect_string("!@#$%^&*()");
}

/// Verifies a string literal containing only spaces.
#[test]
fn test_string_literal_spaces() {
    ShapeTest::new(r#"fn test() -> string { "   " } test()"#).expect_string("   ");
}

/// Verifies a long string literal.
#[test]
fn test_string_literal_long() {
    ShapeTest::new(
        r#"fn test() -> string { "the quick brown fox jumps over the lazy dog" } test()"#,
    )
    .expect_string("the quick brown fox jumps over the lazy dog");
}

// ========================================================================
// 2. String Concatenation (+)
// ========================================================================

/// Verifies concatenation of two strings.
#[test]
fn test_concat_two_strings() {
    ShapeTest::new(
        r#"fn test() -> string { "hello" + " world" }
test()"#,
    )
    .expect_string("hello world");
}

/// Verifies concatenation with empty string on the left.
#[test]
fn test_concat_empty_left() {
    ShapeTest::new(
        r#"fn test() -> string { "" + "hello" }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies concatenation with empty string on the right.
#[test]
fn test_concat_empty_right() {
    ShapeTest::new(
        r#"fn test() -> string { "hello" + "" }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies concatenation of two empty strings.
#[test]
fn test_concat_both_empty() {
    ShapeTest::new(
        r#"fn test() -> string { "" + "" }
test()"#,
    )
    .expect_string("");
}

/// Verifies chaining multiple concatenations.
#[test]
fn test_concat_multiple() {
    ShapeTest::new(
        r#"fn test() -> string { "a" + "b" + "c" + "d" }
test()"#,
    )
    .expect_string("abcd");
}

/// Verifies concatenation with spaces.
#[test]
fn test_concat_with_spaces() {
    ShapeTest::new(
        r#"fn test() -> string { "hello" + " " + "world" }
test()"#,
    )
    .expect_string("hello world");
}

/// Verifies concatenation of variable-held strings.
#[test]
fn test_concat_variable() {
    ShapeTest::new(
        r#"fn test() -> string {
            let a = "foo"
            let b = "bar"
            a + b
        }
test()"#,
    )
    .expect_string("foobar");
}

/// Verifies that concatenation preserves escape sequences.
#[test]
fn test_concat_preserves_escapes() {
    ShapeTest::new(
        r#"fn test() -> bool {
            let s = "a\n" + "b"
            s.contains("\n")
        }
test()"#,
    )
    .expect_bool(true);
}

// ========================================================================
// 13. Escape Sequences
// ========================================================================

/// Verifies newline escape in string length.
#[test]
fn test_escape_newline() {
    ShapeTest::new(
        r#"fn test() -> int { "a\nb".length }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies tab escape in string length.
#[test]
fn test_escape_tab() {
    ShapeTest::new(
        r#"fn test() -> int { "a\tb".length }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies backslash escape in string length.
#[test]
fn test_escape_backslash() {
    ShapeTest::new(
        r#"fn test() -> int { "a\\b".length }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies double-quote escape is contained in string.
#[test]
fn test_escape_double_quote() {
    ShapeTest::new(
        r#"fn test() -> bool { "he\"llo".contains("\"") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies newline escape in contains check.
#[test]
fn test_escape_newline_in_contains() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello\nworld".contains("\n") }
test()"#,
    )
    .expect_bool(true);
}

// ========================================================================
// 10. String Equality
// ========================================================================

/// Verifies equality of identical strings.
#[test]
fn test_string_equal_same() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello" == "hello" }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies inequality of different strings.
#[test]
fn test_string_equal_different() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello" == "world" }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies != operator with different strings.
#[test]
fn test_string_not_equal() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello" != "world" }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies != operator with same strings.
#[test]
fn test_string_not_equal_same() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello" != "hello" }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies case-sensitive string equality.
#[test]
fn test_string_equal_case_sensitive() {
    ShapeTest::new(
        r#"fn test() -> bool { "Hello" == "hello" }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies equality of two empty strings.
#[test]
fn test_string_equal_empty() {
    ShapeTest::new(
        r#"fn test() -> bool { "" == "" }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies inequality of empty vs non-empty strings.
#[test]
fn test_string_not_equal_empty_vs_nonempty() {
    ShapeTest::new(
        r#"fn test() -> bool { "" != "a" }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies string equality with concatenated result.
#[test]
fn test_string_equality_with_concat() {
    ShapeTest::new(
        r#"fn test() -> bool {
            let a = "hel" + "lo"
            a == "hello"
        }
test()"#,
    )
    .expect_bool(true);
}

// ========================================================================
// 11. String Comparison (Lexicographic)
// ========================================================================

/// Verifies lexicographic less-than comparison.
#[test]
fn test_string_less_than() {
    ShapeTest::new(
        r#"fn test() -> bool { "abc" < "abd" }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies lexicographic greater-than comparison.
#[test]
fn test_string_greater_than() {
    ShapeTest::new(
        r#"fn test() -> bool { "b" > "a" }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies less-than-or-equal comparison.
#[test]
fn test_string_less_than_equal() {
    ShapeTest::new(
        r#"fn test() -> bool { "abc" <= "abc" }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies greater-than-or-equal comparison.
#[test]
fn test_string_greater_than_equal() {
    ShapeTest::new(
        r#"fn test() -> bool { "abd" >= "abc" }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies prefix string is less than the full string.
#[test]
fn test_string_compare_prefix() {
    ShapeTest::new(
        r#"fn test() -> bool { "ab" < "abc" }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies string comparison with variables.
#[test]
fn test_string_comparison_with_variables() {
    ShapeTest::new(
        r#"fn test() -> bool {
            let a = "apple"
            let b = "banana"
            a < b
        }
test()"#,
    )
    .expect_bool(true);
}

// ========================================================================
// 22. String Assignment & Variables
// ========================================================================

/// Verifies basic string variable assignment.
#[test]
fn test_string_variable_assignment() {
    ShapeTest::new(
        r#"fn test() -> string {
            let s = "hello"
            s
        }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies storing concatenation result in a variable.
#[test]
fn test_string_concat_variable_result() {
    ShapeTest::new(
        r#"fn test() -> string {
            let a = "foo"
            let b = "bar"
            let c = a + b
            c
        }
test()"#,
    )
    .expect_string("foobar");
}

/// Verifies mutable string variable reassignment.
#[test]
fn test_string_reassignment() {
    ShapeTest::new(
        r#"fn test() -> string {
            let mut s = "hello"
            s = "world"
            s
        }
test()"#,
    )
    .expect_string("world");
}

// ========================================================================
// 23. String in Conditionals
// ========================================================================

/// Verifies string equality used in if condition (true branch).
#[test]
fn test_string_equality_in_if() {
    ShapeTest::new(
        r#"fn test() -> string {
            let s = "hello"
            if s == "hello" {
                "yes"
            } else {
                "no"
            }
        }
test()"#,
    )
    .expect_string("yes");
}

/// Verifies string equality used in if condition (false branch).
#[test]
fn test_string_inequality_in_if() {
    ShapeTest::new(
        r#"fn test() -> string {
            let s = "world"
            if s == "hello" {
                "yes"
            } else {
                "no"
            }
        }
test()"#,
    )
    .expect_string("no");
}

// ========================================================================
// 24. String as Function Parameter/Return
// ========================================================================

/// Verifies passing a string as function parameter.
#[test]
fn test_string_function_parameter() {
    ShapeTest::new(
        r#"
        fn greet(name: string) -> string {
            "hello " + name
        }
        fn test() -> string {
            greet("world")
        }
test()"#,
    )
    .expect_string("hello world");
}

/// Verifies returning a string from a function.
#[test]
fn test_string_returned_from_function() {
    ShapeTest::new(
        r#"
        fn make_greeting() -> string {
            "good morning"
        }
        fn test() -> string {
            make_greeting()
        }
test()"#,
    )
    .expect_string("good morning");
}
