//! Stress tests for string methods: length, split, trim, contains, replace,
//! substring, toUpperCase, toLowerCase, indexOf, charAt, repeat, reverse,
//! padStart, padEnd, isDigit, isAlpha, startsWith, endsWith, codePointAt,
//! toString, join, and chained method calls.

use shape_test::shape_test::ShapeTest;

// ========================================================================
// 3. String Length (.length property)
// ========================================================================

/// Verifies length of empty string.
#[test]
fn test_length_empty() {
    ShapeTest::new(
        r#"fn test() -> int { "".length }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies length of single character string.
#[test]
fn test_length_single_char() {
    ShapeTest::new(
        r#"fn test() -> int { "a".length }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies length of "hello".
#[test]
fn test_length_hello() {
    ShapeTest::new(
        r#"fn test() -> int { "hello".length }
test()"#,
    )
    .expect_number(5.0);
}

/// Verifies length with spaces.
#[test]
fn test_length_with_spaces() {
    ShapeTest::new(
        r#"fn test() -> int { "hello world".length }
test()"#,
    )
    .expect_number(11.0);
}

/// Verifies length from variable.
#[test]
fn test_length_from_variable() {
    ShapeTest::new(
        r#"fn test() -> int {
            let s = "abcdef"
            s.length
        }
test()"#,
    )
    .expect_number(6.0);
}

// ========================================================================
// 4. split()
// ========================================================================

/// Verifies split by comma returns correct count.
#[test]
fn test_split_by_comma() {
    ShapeTest::new(
        r#"fn test() -> int {
            let parts = "a,b,c".split(",")
            parts.length
        }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies first element after split.
#[test]
fn test_split_by_comma_first_element() {
    ShapeTest::new(
        r#"fn test() -> string {
            let parts = "hello,world".split(",")
            parts[0]
        }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies second element after split.
#[test]
fn test_split_by_comma_second_element() {
    ShapeTest::new(
        r#"fn test() -> string {
            let parts = "hello,world".split(",")
            parts[1]
        }
test()"#,
    )
    .expect_string("world");
}

/// Verifies split by space.
#[test]
fn test_split_by_space() {
    ShapeTest::new(
        r#"fn test() -> int {
            let parts = "one two three".split(" ")
            parts.length
        }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies split with no match returns single element.
#[test]
fn test_split_no_match() {
    ShapeTest::new(
        r#"fn test() -> int {
            let parts = "hello".split(",")
            parts.length
        }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies split on empty string.
#[test]
fn test_split_empty_string() {
    ShapeTest::new(
        r#"fn test() -> int {
            let parts = "".split(",")
            parts.length
        }
test()"#,
    )
    .expect_number(1.0);
}

/// Verifies split with multi-char separator.
#[test]
fn test_split_multi_char_separator() {
    ShapeTest::new(
        r#"fn test() -> int {
            let parts = "a::b::c".split("::")
            parts.length
        }
test()"#,
    )
    .expect_number(3.0);
}

// ========================================================================
// 5. trim(), trimStart(), trimEnd()
// ========================================================================

/// Verifies trim removes leading and trailing spaces.
#[test]
fn test_trim_spaces() {
    ShapeTest::new(
        r#"fn test() -> string { "  hello  ".trim() }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies trim on string without whitespace.
#[test]
fn test_trim_no_whitespace() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".trim() }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies trim on string of only spaces.
#[test]
fn test_trim_only_spaces() {
    ShapeTest::new(
        r#"fn test() -> string { "   ".trim() }
test()"#,
    )
    .expect_string("");
}

/// Verifies trim on empty string.
#[test]
fn test_trim_empty() {
    ShapeTest::new(
        r#"fn test() -> string { "".trim() }
test()"#,
    )
    .expect_string("");
}

/// Verifies trim with only leading whitespace.
#[test]
fn test_trim_leading_only() {
    ShapeTest::new(
        r#"fn test() -> string { "  hello".trim() }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies trim with only trailing whitespace.
#[test]
fn test_trim_trailing_only() {
    ShapeTest::new(
        r#"fn test() -> string { "hello  ".trim() }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies trimStart removes only leading whitespace.
#[test]
fn test_trim_start() {
    ShapeTest::new(
        r#"fn test() -> string { "  hello  ".trimStart() }
test()"#,
    )
    .expect_string("hello  ");
}

/// Verifies trimEnd removes only trailing whitespace.
#[test]
fn test_trim_end() {
    ShapeTest::new(
        r#"fn test() -> string { "  hello  ".trimEnd() }
test()"#,
    )
    .expect_string("  hello");
}

// ========================================================================
// 6. contains()
// ========================================================================

/// Verifies contains finds substring.
#[test]
fn test_contains_found() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello world".contains("world") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies contains returns false for missing substring.
#[test]
fn test_contains_not_found() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello world".contains("goodbye") }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies contains with empty search string.
#[test]
fn test_contains_empty_search() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello".contains("") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies contains with full match.
#[test]
fn test_contains_full_match() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello".contains("hello") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies contains is case-sensitive.
#[test]
fn test_contains_case_sensitive() {
    ShapeTest::new(
        r#"fn test() -> bool { "Hello".contains("hello") }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies contains with single char.
#[test]
fn test_contains_single_char() {
    ShapeTest::new(
        r#"fn test() -> bool { "abcdef".contains("d") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies contains at start of string.
#[test]
fn test_contains_at_start() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello world".contains("hello") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies contains at end of string.
#[test]
fn test_contains_at_end() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello world".contains("world") }
test()"#,
    )
    .expect_bool(true);
}

// ========================================================================
// 7. replace()
// ========================================================================

/// Verifies replace with single occurrence.
#[test]
fn test_replace_single_occurrence() {
    ShapeTest::new(
        r#"fn test() -> string { "hello world".replace("world", "rust") }
test()"#,
    )
    .expect_string("hello rust");
}

/// Verifies replace with multiple occurrences.
#[test]
fn test_replace_multiple_occurrences() {
    ShapeTest::new(
        r#"fn test() -> string { "aaa".replace("a", "b") }
test()"#,
    )
    .expect_string("bbb");
}

/// Verifies replace with empty replacement.
#[test]
fn test_replace_with_empty() {
    ShapeTest::new(
        r#"fn test() -> string { "hello world".replace("world", "") }
test()"#,
    )
    .expect_string("hello ");
}

/// Verifies replace with no match.
#[test]
fn test_replace_no_match() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".replace("xyz", "abc") }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies replace with longer replacement.
#[test]
fn test_replace_with_longer() {
    ShapeTest::new(
        r#"fn test() -> string { "hi".replace("hi", "hello") }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies replace with overlapping patterns.
#[test]
fn test_replace_overlapping() {
    ShapeTest::new(
        r#"fn test() -> string { "aaaa".replace("aa", "b") }
test()"#,
    )
    .expect_string("bb");
}

// ========================================================================
// 8. substring()
// ========================================================================

/// Verifies substring with start and end.
#[test]
fn test_substring_with_start_and_end() {
    ShapeTest::new(
        r#"fn test() -> string { "hello world".substring(0, 5) }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies substring of middle portion.
#[test]
fn test_substring_middle() {
    ShapeTest::new(
        r#"fn test() -> string { "hello world".substring(6, 11) }
test()"#,
    )
    .expect_string("world");
}

/// Verifies single character substring.
#[test]
fn test_substring_single_char() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".substring(1, 2) }
test()"#,
    )
    .expect_string("e");
}

/// Verifies substring from start.
#[test]
fn test_substring_from_start() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".substring(0, 3) }
test()"#,
    )
    .expect_string("hel");
}

/// Verifies substring to end with single arg.
#[test]
fn test_substring_to_end_no_second_arg() {
    ShapeTest::new(
        r#"fn test() -> string { "hello world".substring(6) }
test()"#,
    )
    .expect_string("world");
}

/// Verifies empty range substring.
#[test]
fn test_substring_empty_range() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".substring(2, 2) }
test()"#,
    )
    .expect_string("");
}

/// Verifies full string substring.
#[test]
fn test_substring_full_string() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".substring(0, 5) }
test()"#,
    )
    .expect_string("hello");
}

// ========================================================================
// 9. toUpperCase() / toLowerCase()
// ========================================================================

/// Verifies basic toUpperCase.
#[test]
fn test_to_uppercase_basic() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".toUpperCase() }
test()"#,
    )
    .expect_string("HELLO");
}

/// Verifies toUpperCase on already uppercase.
#[test]
fn test_to_uppercase_already_upper() {
    ShapeTest::new(
        r#"fn test() -> string { "HELLO".toUpperCase() }
test()"#,
    )
    .expect_string("HELLO");
}

/// Verifies toUpperCase on mixed case.
#[test]
fn test_to_uppercase_mixed() {
    ShapeTest::new(
        r#"fn test() -> string { "Hello World".toUpperCase() }
test()"#,
    )
    .expect_string("HELLO WORLD");
}

/// Verifies toUpperCase on empty string.
#[test]
fn test_to_uppercase_empty() {
    ShapeTest::new(
        r#"fn test() -> string { "".toUpperCase() }
test()"#,
    )
    .expect_string("");
}

/// Verifies basic toLowerCase.
#[test]
fn test_to_lowercase_basic() {
    ShapeTest::new(
        r#"fn test() -> string { "HELLO".toLowerCase() }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies toLowerCase on already lowercase.
#[test]
fn test_to_lowercase_already_lower() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".toLowerCase() }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies toLowerCase on mixed case.
#[test]
fn test_to_lowercase_mixed() {
    ShapeTest::new(
        r#"fn test() -> string { "Hello World".toLowerCase() }
test()"#,
    )
    .expect_string("hello world");
}

/// Verifies toLowerCase preserves digits.
#[test]
fn test_to_lowercase_with_digits() {
    ShapeTest::new(
        r#"fn test() -> string { "ABC123".toLowerCase() }
test()"#,
    )
    .expect_string("abc123");
}

// ========================================================================
// 12. indexOf()
// ========================================================================

/// Verifies indexOf finds substring.
#[test]
fn test_index_of_found() {
    ShapeTest::new(
        r#"fn test() -> int { "hello world".indexOf("world") }
test()"#,
    )
    .expect_number(6.0);
}

/// Verifies indexOf returns -1 for missing substring.
#[test]
fn test_index_of_not_found() {
    ShapeTest::new(
        r#"fn test() -> int { "hello".indexOf("xyz") }
test()"#,
    )
    .expect_number(-1.0);
}

/// Verifies indexOf at start.
#[test]
fn test_index_of_at_start() {
    ShapeTest::new(
        r#"fn test() -> int { "hello".indexOf("hel") }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies indexOf at end.
#[test]
fn test_index_of_at_end() {
    ShapeTest::new(
        r#"fn test() -> int { "hello".indexOf("llo") }
test()"#,
    )
    .expect_number(2.0);
}

/// Verifies indexOf for single char.
#[test]
fn test_index_of_single_char() {
    ShapeTest::new(
        r#"fn test() -> int { "abcdef".indexOf("d") }
test()"#,
    )
    .expect_number(3.0);
}

/// Verifies indexOf with empty string returns 0.
#[test]
fn test_index_of_empty_string() {
    ShapeTest::new(
        r#"fn test() -> int { "hello".indexOf("") }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies indexOf returns first occurrence.
#[test]
fn test_index_of_first_occurrence() {
    ShapeTest::new(
        r#"fn test() -> int { "abcabc".indexOf("bc") }
test()"#,
    )
    .expect_number(1.0);
}

// ========================================================================
// 14. startsWith() / endsWith()
// ========================================================================

/// Verifies startsWith returns true.
#[test]
fn test_starts_with_true() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello world".startsWith("hello") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies startsWith returns false.
#[test]
fn test_starts_with_false() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello world".startsWith("world") }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies startsWith with empty string.
#[test]
fn test_starts_with_empty() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello".startsWith("") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies startsWith with full match.
#[test]
fn test_starts_with_full_match() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello".startsWith("hello") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies endsWith returns true.
#[test]
fn test_ends_with_true() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello world".endsWith("world") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies endsWith returns false.
#[test]
fn test_ends_with_false() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello world".endsWith("hello") }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies endsWith with empty string.
#[test]
fn test_ends_with_empty() {
    ShapeTest::new(
        r#"fn test() -> bool { "hello".endsWith("") }
test()"#,
    )
    .expect_bool(true);
}

// ========================================================================
// 15. charAt()
// ========================================================================

/// Verifies charAt at first position.
#[test]
fn test_char_at_first() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".charAt(0) }
test()"#,
    )
    .expect_string("h");
}

/// Verifies charAt at middle position.
#[test]
fn test_char_at_middle() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".charAt(2) }
test()"#,
    )
    .expect_string("l");
}

/// Verifies charAt at last position.
#[test]
fn test_char_at_last() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".charAt(4) }
test()"#,
    )
    .expect_string("o");
}

/// Verifies charAt out of bounds returns empty string.
#[test]
fn test_char_at_out_of_bounds() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".charAt(10) }
test()"#,
    )
    .expect_string("");
}

// ========================================================================
// 16. repeat()
// ========================================================================

/// Verifies basic repeat.
#[test]
fn test_repeat_basic() {
    ShapeTest::new(
        r#"fn test() -> string { "ab".repeat(3) }
test()"#,
    )
    .expect_string("ababab");
}

/// Verifies repeat zero times.
#[test]
fn test_repeat_zero() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".repeat(0) }
test()"#,
    )
    .expect_string("");
}

/// Verifies repeat one time.
#[test]
fn test_repeat_one() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".repeat(1) }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies repeat single char.
#[test]
fn test_repeat_single_char() {
    ShapeTest::new(
        r#"fn test() -> string { "x".repeat(5) }
test()"#,
    )
    .expect_string("xxxxx");
}

// ========================================================================
// 17. reverse()
// ========================================================================

/// Verifies basic string reverse.
#[test]
fn test_reverse_basic() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".reverse() }
test()"#,
    )
    .expect_string("olleh");
}

/// Verifies reverse of palindrome.
#[test]
fn test_reverse_palindrome() {
    ShapeTest::new(
        r#"fn test() -> string { "racecar".reverse() }
test()"#,
    )
    .expect_string("racecar");
}

/// Verifies reverse of single char.
#[test]
fn test_reverse_single_char() {
    ShapeTest::new(
        r#"fn test() -> string { "a".reverse() }
test()"#,
    )
    .expect_string("a");
}

/// Verifies reverse of empty string.
#[test]
fn test_reverse_empty() {
    ShapeTest::new(
        r#"fn test() -> string { "".reverse() }
test()"#,
    )
    .expect_string("");
}

// ========================================================================
// 18. padStart() / padEnd()
// ========================================================================

/// Verifies basic padStart.
#[test]
fn test_pad_start_basic() {
    ShapeTest::new(
        r#"fn test() -> string { "42".padStart(5, "0") }
test()"#,
    )
    .expect_string("00042");
}

/// Verifies padStart when no padding needed.
#[test]
fn test_pad_start_no_padding_needed() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".padStart(3, "x") }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies padStart with default space padding.
#[test]
fn test_pad_start_default_space() {
    ShapeTest::new(
        r#"fn test() -> string { "hi".padStart(5) }
test()"#,
    )
    .expect_string("   hi");
}

/// Verifies basic padEnd.
#[test]
fn test_pad_end_basic() {
    ShapeTest::new(
        r#"fn test() -> string { "hi".padEnd(5, ".") }
test()"#,
    )
    .expect_string("hi...");
}

/// Verifies padEnd when no padding needed.
#[test]
fn test_pad_end_no_padding_needed() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".padEnd(3, "x") }
test()"#,
    )
    .expect_string("hello");
}

/// Verifies padEnd with default space padding.
#[test]
fn test_pad_end_default_space() {
    ShapeTest::new(
        r#"fn test() -> string { "hi".padEnd(5) }
test()"#,
    )
    .expect_string("hi   ");
}

// ========================================================================
// 19. isDigit() / isAlpha()
// ========================================================================

/// Verifies isDigit returns true for all digits.
#[test]
fn test_is_digit_true() {
    ShapeTest::new(
        r#"fn test() -> bool { "12345".isDigit() }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies isDigit returns false for mixed.
#[test]
fn test_is_digit_false() {
    ShapeTest::new(
        r#"fn test() -> bool { "123a5".isDigit() }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies isDigit returns false for empty.
#[test]
fn test_is_digit_empty() {
    ShapeTest::new(
        r#"fn test() -> bool { "".isDigit() }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies isAlpha returns true for all alpha.
#[test]
fn test_is_alpha_true() {
    ShapeTest::new(
        r#"fn test() -> bool { "abcDEF".isAlpha() }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies isAlpha returns false for mixed.
#[test]
fn test_is_alpha_false() {
    ShapeTest::new(
        r#"fn test() -> bool { "abc123".isAlpha() }
test()"#,
    )
    .expect_bool(false);
}

/// Verifies isAlpha returns false for empty.
#[test]
fn test_is_alpha_empty() {
    ShapeTest::new(
        r#"fn test() -> bool { "".isAlpha() }
test()"#,
    )
    .expect_bool(false);
}

// ========================================================================
// 20. Chained Methods
// ========================================================================

/// Verifies trim then toUpperCase chain.
#[test]
fn test_chain_trim_to_upper() {
    ShapeTest::new(
        r#"fn test() -> string { "  hello  ".trim().toUpperCase() }
test()"#,
    )
    .expect_string("HELLO");
}

/// Verifies toLowerCase then contains chain.
#[test]
fn test_chain_to_lower_contains() {
    ShapeTest::new(
        r#"fn test() -> bool { "HELLO WORLD".toLowerCase().contains("hello") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies replace then toUpperCase chain.
#[test]
fn test_chain_replace_to_upper() {
    ShapeTest::new(
        r#"fn test() -> string { "hello world".replace("world", "rust").toUpperCase() }
test()"#,
    )
    .expect_string("HELLO RUST");
}

/// Verifies trim then replace chain.
#[test]
fn test_chain_trim_replace() {
    ShapeTest::new(
        r#"fn test() -> string { "  hello world  ".trim().replace("world", "shape") }
test()"#,
    )
    .expect_string("hello shape");
}

/// Verifies substring then toUpperCase chain.
#[test]
fn test_chain_substring_to_upper() {
    ShapeTest::new(
        r#"fn test() -> string { "hello world".substring(0, 5).toUpperCase() }
test()"#,
    )
    .expect_string("HELLO");
}

// ========================================================================
// 25. codePointAt()
// ========================================================================

/// Verifies codePointAt for ASCII 'A'.
#[test]
fn test_code_point_at_ascii() {
    ShapeTest::new(
        r#"fn test() -> int { "A".codePointAt(0) }
test()"#,
    )
    .expect_number(65.0);
}

/// Verifies codePointAt for lowercase 'a'.
#[test]
fn test_code_point_at_lowercase_a() {
    ShapeTest::new(
        r#"fn test() -> int { "a".codePointAt(0) }
test()"#,
    )
    .expect_number(97.0);
}

/// Verifies codePointAt out of bounds returns -1.
#[test]
fn test_code_point_at_out_of_bounds() {
    ShapeTest::new(
        r#"fn test() -> int { "a".codePointAt(5) }
test()"#,
    )
    .expect_number(-1.0);
}

// ========================================================================
// 26. toString() on strings
// ========================================================================

/// Verifies toString on string returns the same string.
#[test]
fn test_to_string_on_string() {
    ShapeTest::new(
        r#"fn test() -> string { "hello".toString() }
test()"#,
    )
    .expect_string("hello");
}

// ========================================================================
// 27. Complex / Edge-Case Tests
// ========================================================================

/// Verifies split then join.
#[test]
fn test_split_then_join() {
    ShapeTest::new(
        r#"fn test() -> string {
            let parts = "a,b,c".split(",")
            parts.join("-")
        }
test()"#,
    )
    .expect_string("a-b-c");
}

/// Verifies chained replace calls.
#[test]
fn test_replace_chain() {
    ShapeTest::new(
        r#"fn test() -> string {
            "aabbcc".replace("a", "x").replace("b", "y")
        }
test()"#,
    )
    .expect_string("xxyycc");
}

/// Verifies contains after replace.
#[test]
fn test_contains_after_replace() {
    ShapeTest::new(
        r#"fn test() -> bool {
            "hello world".replace("world", "rust").contains("rust")
        }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies length after concatenation.
#[test]
fn test_length_after_concat() {
    ShapeTest::new(
        r#"fn test() -> int {
            let s = "hello" + " " + "world"
            s.length
        }
test()"#,
    )
    .expect_number(11.0);
}

/// Verifies indexOf after toLowerCase.
#[test]
fn test_index_of_after_to_lower() {
    ShapeTest::new(
        r#"fn test() -> int {
            "HELLO WORLD".toLowerCase().indexOf("world")
        }
test()"#,
    )
    .expect_number(6.0);
}

/// Verifies startsWith after trim.
#[test]
fn test_starts_with_after_trim() {
    ShapeTest::new(
        r#"fn test() -> bool {
            "  hello world".trim().startsWith("hello")
        }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies endsWith after trim.
#[test]
fn test_ends_with_after_trim() {
    ShapeTest::new(
        r#"fn test() -> bool {
            "hello world  ".trim().endsWith("world")
        }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies charAt after reverse.
#[test]
fn test_char_at_after_reverse() {
    ShapeTest::new(
        r#"fn test() -> string { "abc".reverse().charAt(0) }
test()"#,
    )
    .expect_string("c");
}

/// Verifies repeat then length.
#[test]
fn test_repeat_then_length() {
    ShapeTest::new(
        r#"fn test() -> int { "ab".repeat(4).length }
test()"#,
    )
    .expect_number(8.0);
}

/// Verifies padStart then length.
#[test]
fn test_pad_start_then_length() {
    ShapeTest::new(
        r#"fn test() -> int { "hi".padStart(10, "0").length }
test()"#,
    )
    .expect_number(10.0);
}

/// Verifies split count elements.
#[test]
fn test_split_count_elements() {
    ShapeTest::new(
        r#"fn test() -> int {
            "one,two,three,four,five".split(",").length
        }
test()"#,
    )
    .expect_number(5.0);
}

/// Verifies string concatenation in loop.
#[test]
fn test_string_in_loop_concat() {
    ShapeTest::new(
        r#"fn test() -> string {
            let mut s = ""
            for i in range(0, 5) {
                s = s + "a"
            }
            s
        }
test()"#,
    )
    .expect_string("aaaaa");
}

/// Verifies substring then contains.
#[test]
fn test_substring_then_contains() {
    ShapeTest::new(
        r#"fn test() -> bool {
            "hello world".substring(0, 5).contains("hell")
        }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies replace empty string with content.
#[test]
fn test_replace_empty_with_content() {
    ShapeTest::new(
        r#"fn test() -> string {
            "hello".replace("", "-")
        }
test()"#,
    )
    .expect_string("-h-e-l-l-o-");
}

/// Verifies trimStart with tab and leading whitespace.
#[test]
fn test_trim_start_only_leading() {
    ShapeTest::new(
        r#"fn test() -> string { "\t hello ".trimStart() }
test()"#,
    )
    .expect_string("hello ");
}

/// Verifies trimEnd with tab and trailing whitespace.
#[test]
fn test_trim_end_only_trailing() {
    ShapeTest::new(
        r#"fn test() -> string { " hello \t".trimEnd() }
test()"#,
    )
    .expect_string(" hello");
}

/// Verifies split with single char separator and index access.
#[test]
fn test_split_single_char_separator() {
    ShapeTest::new(
        r#"fn test() -> string {
            let parts = "a-b-c".split("-")
            parts[2]
        }
test()"#,
    )
    .expect_string("c");
}

/// Verifies multiple string methods in function.
#[test]
fn test_multiple_string_methods_in_function() {
    ShapeTest::new(
        r#"fn test() -> string {
            let input = "  Hello, World!  "
            let trimmed = input.trim()
            let lower = trimmed.toLowerCase()
            let replaced = lower.replace(",", "")
            replaced
        }
test()"#,
    )
    .expect_string("hello world!");
}

/// Verifies indexOf with multi-char needle.
#[test]
fn test_index_of_multichar_needle() {
    ShapeTest::new(
        r#"fn test() -> int {
            "the quick brown fox".indexOf("quick")
        }
test()"#,
    )
    .expect_number(4.0);
}

/// Verifies padEnd with multi-char fill.
#[test]
fn test_pad_end_with_multichar_fill() {
    ShapeTest::new(
        r#"fn test() -> string { "x".padEnd(5, "ab") }
test()"#,
    )
    .expect_string("xabab");
}

/// Verifies padStart with multi-char fill.
#[test]
fn test_pad_start_with_multichar_fill() {
    ShapeTest::new(
        r#"fn test() -> string { "x".padStart(5, "ab") }
test()"#,
    )
    .expect_string("ababx");
}

/// Verifies string length after replace.
#[test]
fn test_string_length_after_replace() {
    ShapeTest::new(
        r#"fn test() -> int {
            "hello".replace("l", "xx").length
        }
test()"#,
    )
    .expect_number(7.0);
}

/// Verifies various methods on empty string.
#[test]
fn test_empty_string_methods() {
    ShapeTest::new(
        r#"fn test() -> int { "".length }
test()"#,
    )
    .expect_number(0.0);
}

/// Verifies empty string contains empty string.
#[test]
fn test_empty_string_contains_empty() {
    ShapeTest::new(
        r#"fn test() -> bool { "".contains("") }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies split on empty string returns empty first element.
#[test]
fn test_empty_string_split() {
    ShapeTest::new(
        r#"fn test() -> string {
            let parts = "".split(",")
            parts[0]
        }
test()"#,
    )
    .expect_string("");
}
