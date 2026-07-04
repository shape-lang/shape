//! snake_case string-method aliases documented in the book
//! (`shape-web/.../fundamentals/strings.mdx` §Methods).
//!
//! The book lists snake_case aliases that must resolve to the *same*
//! handlers as their camelCase equivalents:
//!   - `to_upper_case()`  ⇄ `toUpperCase()`
//!   - `to_lower_case()`  ⇄ `toLowerCase()`
//!   - `trim_start()`     ⇄ `trimStart()`
//!   - `trim_end()`       ⇄ `trimEnd()`
//!
//! Before the fix the type-checker method table (and the UFCS builtin
//! recognizer) only knew the camelCase forms, so a snake_case call failed
//! type-checking with "Method '...' not found on type 'string'" long
//! before reaching the PHF dispatch registry (which already aliased them).

use super::test_utils::eval;

#[test]
fn to_upper_case_snake_alias_resolves() {
    assert_eq!(eval(r#""Hi".to_upper_case()"#).as_str(), Some("HI"));
}

#[test]
fn to_lower_case_snake_alias_resolves() {
    assert_eq!(eval(r#""HI".to_lower_case()"#).as_str(), Some("hi"));
}

#[test]
fn trim_start_snake_alias_resolves() {
    assert_eq!(eval(r#""  x  ".trim_start()"#).as_str(), Some("x  "));
}

#[test]
fn trim_end_snake_alias_resolves() {
    assert_eq!(eval(r#""  x  ".trim_end()"#).as_str(), Some("  x"));
}

#[test]
fn camel_case_forms_still_resolve() {
    // The fix must not displace the original camelCase spelling.
    assert_eq!(eval(r#""Hi".toUpperCase()"#).as_str(), Some("HI"));
    assert_eq!(eval(r#""HI".toLowerCase()"#).as_str(), Some("hi"));
    assert_eq!(eval(r#""  x  ".trimStart()"#).as_str(), Some("x  "));
    assert_eq!(eval(r#""  x  ".trimEnd()"#).as_str(), Some("  x"));
}

#[test]
fn snake_and_camel_aliases_produce_identical_results() {
    assert_eq!(
        eval(r#""MixedCase".to_upper_case()"#).as_str(),
        eval(r#""MixedCase".toUpperCase()"#).as_str(),
    );
    assert_eq!(
        eval(r#""MixedCase".to_lower_case()"#).as_str(),
        eval(r#""MixedCase".toLowerCase()"#).as_str(),
    );
}
