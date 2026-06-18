//! Character-literal tests — STAGE OP3.
//!
//! The book (`fundamentals/operators.mdx` "Character Literals") gives Shape
//! NO first-class `char` type: a single-quote literal `'a'` evaluates to its
//! integer Unicode code point — the interop / parsing escape hatch. `'A'` IS
//! the int 65, usable anywhere an `int` is. There is no char<->string
//! coercion: to get a 1-char STRING use a string literal `"a"` or `s[i]`.
//!
//! Pre-fix the implementation typed `Literal::Char` as a distinct, self-
//! contradictory `char` type (rejected against `int`, rendered as a string by
//! `print`, rejected against `string` by `+`). These tests pin the
//! int-codepoint model on BOTH the VM and JIT paths.

use shape_test::shape_test::ShapeTest;

/// `'A'` is the int 65.
#[test]
fn test_char_literal_is_int_codepoint() {
    ShapeTest::new("fn test() -> int { 'A' }\ntest()").expect_number(65.0);
}

/// A char literal binds to an `int` annotation (no distinct char type).
#[test]
fn test_char_literal_assignable_to_int() {
    ShapeTest::new("fn test() -> int { let c: int = 'A'\n c }\ntest()").expect_number(65.0);
}

/// Char literal participates in int arithmetic: `'a' + 1` == 98.
#[test]
fn test_char_literal_int_arithmetic() {
    ShapeTest::new("fn test() -> int { 'a' + 1 }\ntest()").expect_number(98.0);
}

/// Digit char literal `'0'` is the int 48.
#[test]
fn test_char_literal_digit_zero() {
    ShapeTest::new("fn test() -> int { '0' }\ntest()").expect_number(48.0);
}

/// Escape char literal `'\n'` is the int 10.
#[test]
fn test_char_literal_newline_escape() {
    ShapeTest::new("fn test() -> int { '\\n' }\ntest()").expect_number(10.0);
}

/// A multi-byte Unicode char literal is its full code point (😀 == 128512).
#[test]
fn test_char_literal_unicode_codepoint() {
    ShapeTest::new("fn test() -> int { '😀' }\ntest()").expect_number(128512.0);
}

/// A char literal is usable as an array index: `arr['A' - 'A']` == arr[0].
#[test]
fn test_char_literal_as_array_index() {
    ShapeTest::new(
        r#"fn test() -> int {
            let arr: Array<int> = [10, 20, 30]
            arr['A' - 'A']
        }
test()"#,
    )
    .expect_number(10.0);
}

/// `print('A')` emits the codepoint "65" (the documented visible behavior
/// change — char literals are ints, NOT 1-char strings).
#[test]
fn test_char_literal_print_is_codepoint() {
    ShapeTest::new("print('A')").expect_output("65");
}
