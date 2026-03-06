//! String operation tests — concatenation, methods, f-strings, comparisons.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 3. String Operations (20 tests)
// =========================================================================

#[test]
fn test_string_concatenation_basic() {
    ShapeTest::new(
        r#"
        "hello" + " " + "world"
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn test_string_length() {
    ShapeTest::new(
        r#"
        "hello".length
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_string_split_basic() {
    ShapeTest::new(
        r#"
        let parts = "a,b,c".split(",")
        parts.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_string_split_first_element() {
    ShapeTest::new(
        r#"
        let parts = "hello world".split(" ")
        parts[0]
    "#,
    )
    .expect_string("hello");
}

#[test]
fn test_string_join_array() {
    ShapeTest::new(
        r#"
        ["a", "b", "c"].join(",")
    "#,
    )
    .expect_string("a,b,c");
}

#[test]
fn test_string_join_with_space() {
    ShapeTest::new(
        r#"
        ["hello", "world"].join(" ")
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn test_string_contains_true() {
    ShapeTest::new(
        r#"
        "hello world".contains("world")
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_string_contains_false() {
    ShapeTest::new(
        r#"
        "hello world".contains("xyz")
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_string_substring_basic() {
    ShapeTest::new(
        r#"
        "hello".substring(1, 3)
    "#,
    )
    .expect_string("el");
}

#[test]
fn test_string_substring_from_start() {
    ShapeTest::new(
        r#"
        "hello world".substring(0, 5)
    "#,
    )
    .expect_string("hello");
}

#[test]
fn test_string_to_uppercase() {
    ShapeTest::new(
        r#"
        "hello".toUpperCase()
    "#,
    )
    .expect_string("HELLO");
}

#[test]
fn test_string_to_lowercase() {
    ShapeTest::new(
        r#"
        "HELLO".toLowerCase()
    "#,
    )
    .expect_string("hello");
}

#[test]
fn test_string_trim() {
    ShapeTest::new(
        r#"
        "  hello  ".trim()
    "#,
    )
    .expect_string("hello");
}

#[test]
fn test_string_replace_basic() {
    ShapeTest::new(
        r#"
        "foo bar foo".replace("foo", "baz")
    "#,
    )
    .expect_string("baz bar baz");
}

#[test]
fn test_string_replace_single_char() {
    ShapeTest::new(
        r#"
        "aabbcc".replace("b", "x")
    "#,
    )
    .expect_string("aaxxcc");
}

#[test]
fn test_string_fstring_basic() {
    ShapeTest::new(
        r#"
        let name = "world"
        f"hello {name}"
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn test_string_fstring_with_expression() {
    ShapeTest::new(
        r#"
        f"result = {1 + 2}"
    "#,
    )
    .expect_string("result = 3");
}

#[test]
fn test_string_fstring_multiple_interpolations() {
    ShapeTest::new(
        r#"
        let a = "foo"
        let b = "bar"
        f"{a} and {b}"
    "#,
    )
    .expect_string("foo and bar");
}

#[test]
fn test_string_comparison_equality() {
    ShapeTest::new(
        r#"
        "hello" == "hello"
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_string_comparison_inequality() {
    ShapeTest::new(
        r#"
        "hello" != "world"
    "#,
    )
    .expect_bool(true);
}
