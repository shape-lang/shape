//! Stress tests for Option (Some/None), null coalesce (??), and null comparison patterns.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 3: Some creation
// =============================================================================

/// Some(42) is identity in Shape — returns the value as-is.
#[test]
fn some_int() {
    ShapeTest::new("Some(42)").expect_number(42.0);
}

/// Some wraps string.
#[test]
fn some_string() {
    ShapeTest::new(r#"Some("hello")"#).expect_string("hello");
}

/// Some wraps bool.
#[test]
fn some_bool() {
    ShapeTest::new("Some(true)").expect_bool(true);
}

/// Some wraps float.
#[test]
fn some_float() {
    ShapeTest::new("Some(2.5)").expect_number(2.5);
}

/// Some wraps zero.
#[test]
fn some_zero() {
    ShapeTest::new("Some(0)").expect_number(0.0);
}

/// Some wraps negative.
#[test]
fn some_negative() {
    ShapeTest::new("Some(-100)").expect_number(-100.0);
}

// =============================================================================
// SECTION 4: None / null
// =============================================================================

/// None literal is none.
#[test]
fn null_literal_is_none() {
    ShapeTest::new("None").expect_none();
}

/// Null from variable.
#[test]
fn null_from_variable() {
    ShapeTest::new("let x = None\nx").expect_none();
}

/// Null equality both null.
#[test]
fn null_equality_both_null() {
    ShapeTest::new("None == None").expect_bool(true);
}

/// Null not equal to int.
#[test]
fn null_not_equal_to_int() {
    ShapeTest::new("None != 1").expect_bool(true);
}

/// Null not equal to zero.
#[test]
fn null_not_equal_to_zero() {
    ShapeTest::new("None != 0").expect_bool(true);
}

/// Null not equal to false.
#[test]
fn null_not_equal_to_false() {
    ShapeTest::new("None != false").expect_bool(true);
}

/// Null not equal to string.
#[test]
fn null_not_equal_to_string() {
    ShapeTest::new(r#"None != "hello""#).expect_bool(true);
}

/// Null eq int is false.
#[test]
fn null_eq_int_is_false() {
    ShapeTest::new("None == 1").expect_bool(false);
}

/// Null eq false is false.
#[test]
fn null_eq_false_is_false() {
    ShapeTest::new("None == false").expect_bool(false);
}

/// Null eq empty string is false.
#[test]
fn null_eq_empty_string_is_false() {
    ShapeTest::new(r#"None == """#).expect_bool(false);
}

// =============================================================================
// SECTION 5: Null coalesce (??)
// =============================================================================

/// Null coalesce null gives fallback.
#[test]
fn null_coalesce_null_gives_fallback() {
    ShapeTest::new("None ?? 42").expect_number(42.0);
}

/// Null coalesce value gives value.
#[test]
fn null_coalesce_value_gives_value() {
    ShapeTest::new("10 ?? 42").expect_number(10.0);
}

/// Null coalesce zero gives zero.
#[test]
fn null_coalesce_zero_gives_zero() {
    ShapeTest::new("0 ?? 42").expect_number(0.0);
}

/// Null coalesce false gives false.
#[test]
fn null_coalesce_false_gives_false() {
    ShapeTest::new("false ?? true").expect_bool(false);
}

/// Null coalesce string value.
#[test]
fn null_coalesce_string_value() {
    ShapeTest::new(r#""hi" ?? "fallback""#).expect_string("hi");
}

/// Null coalesce null to string fallback.
#[test]
fn null_coalesce_null_to_string_fallback() {
    ShapeTest::new(r#"None ?? "default""#).expect_string("default");
}

/// Null coalesce with variable.
#[test]
fn null_coalesce_with_variable() {
    ShapeTest::new("let x = None\nx ?? 99").expect_number(99.0);
}

/// Null coalesce with non-null variable.
#[test]
fn null_coalesce_with_non_null_variable() {
    ShapeTest::new("let x = 7\nx ?? 99").expect_number(7.0);
}

// =============================================================================
// SECTION 6: Chained null coalesce (??)
// =============================================================================

/// Chained coalesce first null second null.
#[test]
fn chained_coalesce_first_null_second_null() {
    ShapeTest::new("None ?? None ?? 100").expect_number(100.0);
}

/// Chained coalesce first null second value.
#[test]
fn chained_coalesce_first_null_second_value() {
    ShapeTest::new("None ?? 50 ?? 100").expect_number(50.0);
}

/// Chained coalesce first value.
#[test]
fn chained_coalesce_first_value() {
    ShapeTest::new("10 ?? 50 ?? 100").expect_number(10.0);
}

/// Chained coalesce with variables.
#[test]
fn chained_coalesce_with_variables() {
    ShapeTest::new("let a = None\nlet b = None\nlet c = 77\na ?? b ?? c").expect_number(77.0);
}

/// Chained coalesce four levels.
#[test]
fn chained_coalesce_four_levels() {
    ShapeTest::new("None ?? None ?? None ?? 1").expect_number(1.0);
}

// =============================================================================
// SECTION 8: Match on Option (Some/None arms)
// =============================================================================

/// Match some value.
#[test]
fn match_some_value() {
    ShapeTest::new("let x = Some(42)\nmatch x { Some(v) => v, None => -1 }").expect_number(42.0);
}

/// Match null fallback.
#[test]
fn match_null_fallback() {
    ShapeTest::new("let x = None\nmatch x { Some(v) => v, None => -1 }").expect_number(-1.0);
}

/// Match some string value.
#[test]
fn match_some_string_value() {
    ShapeTest::new(
        r#"let x = Some("hello")
match x { Some(v) => v, None => "default" }"#,
    )
    .expect_string("hello");
}

/// Match null string fallback.
#[test]
fn match_null_string_fallback() {
    ShapeTest::new(
        r#"let x = None
match x { Some(v) => v, None => "default" }"#,
    )
    .expect_string("default");
}

// =============================================================================
// SECTION 10: Option from function
// =============================================================================

/// Function returning Some value.
#[test]
fn fn_returning_some_value() {
    ShapeTest::new("fn test() -> int { let x = Some(55)\nx ?? 0 }\ntest()").expect_number(55.0);
}

/// Function returning null.
#[test]
fn fn_returning_null() {
    ShapeTest::new("fn test() -> int { let x = None\nx ?? 99 }\ntest()").expect_number(99.0);
}

/// Function conditional some or null — some path.
#[test]
fn fn_conditional_some_or_null() {
    ShapeTest::new(
        "fn maybe(flag: bool) { if flag { return 42 }\nreturn None }\nfn test() -> int { let x = maybe(true)\nx ?? 0 }\ntest()",
    )
    .expect_number(42.0);
}

/// Function conditional null path.
#[test]
fn fn_conditional_null_path() {
    ShapeTest::new(
        "fn maybe(flag: bool) { if flag { return 42 }\nreturn None }\nfn test() -> int { let x = maybe(false)\nx ?? 0 }\ntest()",
    )
    .expect_number(0.0);
}

// =============================================================================
// SECTION 13: Default values with ?? in functions
// =============================================================================

/// Default value pattern null param.
#[test]
fn default_value_pattern_null_param() {
    ShapeTest::new("fn test() -> int { let x = None\nlet val = x ?? 10\nval }\ntest()")
        .expect_number(10.0);
}

/// Default value pattern non-null param.
#[test]
fn default_value_pattern_non_null_param() {
    ShapeTest::new("fn test() -> int { let x = 5\nlet val = x ?? 10\nval }\ntest()")
        .expect_number(5.0);
}

/// Default value string fallback.
#[test]
fn default_value_string_fallback() {
    ShapeTest::new(
        r#"fn test() -> string { let x = None
let val = x ?? "unknown"
val }
test()"#,
    )
    .expect_string("unknown");
}

// =============================================================================
// SECTION 16: Boolean checks on Option
// =============================================================================

/// Option not null check with value.
#[test]
fn option_not_null_check_with_value() {
    ShapeTest::new("fn test() -> bool { let x = 42\nx != None }\ntest()").expect_bool(true);
}

/// Option not null check with null.
#[test]
fn option_not_null_check_with_null() {
    ShapeTest::new("fn test() -> bool { let x = None\nx != None }\ntest()").expect_bool(false);
}

/// Option eq null check with null.
#[test]
fn option_eq_null_check_with_null() {
    ShapeTest::new("fn test() -> bool { let x = None\nx == None }\ntest()").expect_bool(true);
}

/// Option eq null check with value.
#[test]
fn option_eq_null_check_with_value() {
    ShapeTest::new("fn test() -> bool { let x = 42\nx == None }\ntest()").expect_bool(false);
}

/// If not null then use value.
#[test]
fn if_not_null_then_use_value() {
    ShapeTest::new("fn test() -> int { let x = 10\nif x != None { x } else { 0 } }\ntest()")
        .expect_number(10.0);
}

/// If null then default.
#[test]
fn if_null_then_default() {
    ShapeTest::new("fn test() -> int { let x = None\nif x != None { 999 } else { 0 } }\ntest()")
        .expect_number(0.0);
}

// =============================================================================
// SECTION 19: Null coalesce with expressions
// =============================================================================

/// Null coalesce with arithmetic fallback.
#[test]
fn null_coalesce_with_arithmetic_fallback() {
    ShapeTest::new("None ?? (2 + 3)").expect_number(5.0);
}

/// Null coalesce with arithmetic lhs.
#[test]
fn null_coalesce_with_arithmetic_lhs() {
    ShapeTest::new("(1 + 2) ?? 99").expect_number(3.0);
}

/// Null coalesce in let binding.
#[test]
fn null_coalesce_in_let_binding() {
    ShapeTest::new("let a = None\nlet b = a ?? 42\nb").expect_number(42.0);
}

// =============================================================================
// SECTION 22: Null in various contexts
// =============================================================================

/// Null assigned to variable.
#[test]
fn null_assigned_to_variable() {
    ShapeTest::new("let x = None\nx").expect_none();
}

/// Null reassigned.
#[test]
fn null_reassigned() {
    ShapeTest::new("let mut x = 42\nx = None\nx").expect_none();
}

/// Variable starts null then assigned.
#[test]
fn variable_starts_null_then_assigned() {
    ShapeTest::new("let mut x = None\nx = 10\nx").expect_number(10.0);
}

/// Null in array.
#[test]
fn null_in_array() {
    ShapeTest::new("let arr = [1, None, 3]\narr[1]").expect_none();
}

// =============================================================================
// SECTION 24: Null coalesce with function calls
// =============================================================================

/// Null coalesce with fn returning null.
#[test]
fn null_coalesce_with_fn_returning_null() {
    ShapeTest::new("fn get_val() { return None }\nfn test() -> int { get_val() ?? 42 }\ntest()")
        .expect_number(42.0);
}

/// Null coalesce with fn returning value.
#[test]
fn null_coalesce_with_fn_returning_value() {
    ShapeTest::new(
        "fn get_val() -> int { return 10 }\nfn test() -> int { get_val() ?? 42 }\ntest()",
    )
    .expect_number(10.0);
}

// =============================================================================
// SECTION 27: Mixed Ok/Some/null patterns
// =============================================================================

/// Ok inside null coalesce.
#[test]
fn ok_inside_null_coalesce() {
    ShapeTest::new("match (Ok(42) ?? 0) { Ok(v) => v, Err(e) => -1 }").expect_number(42.0);
}

/// Null coalesce then match.
#[test]
fn null_coalesce_then_match() {
    ShapeTest::new("fn test() -> int { let x = None\nlet y = x ?? Ok(42)\nmatch y { Ok(v) => v, Err(e) => -1 } }\ntest()")
        .expect_number(42.0);
}

// =============================================================================
// SECTION 28: Edge cases
// =============================================================================

/// Ok wrapping null.
#[test]
fn ok_wrapping_null() {
    ShapeTest::new("match Ok(None) { Ok(v) => v ?? 99, Err(e) => -1 }").expect_number(99.0);
}

/// Match ok null inner.
#[test]
fn match_ok_null_inner() {
    ShapeTest::new("let x = Ok(None)\nmatch x { Ok(v) => v ?? 99, Err(e) => -1 }")
        .expect_number(99.0);
}

/// Null coalesce on false does not trigger.
#[test]
fn null_coalesce_on_false_does_not_trigger() {
    ShapeTest::new("false ?? true").expect_bool(false);
}

/// Null coalesce on zero does not trigger.
#[test]
fn null_coalesce_on_zero_does_not_trigger() {
    ShapeTest::new("0 ?? 999").expect_number(0.0);
}

/// Null coalesce on empty string does not trigger.
#[test]
fn null_coalesce_on_empty_string_does_not_trigger() {
    ShapeTest::new(r#""" ?? "fallback""#).expect_string("");
}

// =============================================================================
// SECTION 30: Null coalesce with different types
// =============================================================================

/// Null coalesce bool fallback.
#[test]
fn null_coalesce_bool_fallback() {
    ShapeTest::new("None ?? true").expect_bool(true);
}

/// Null coalesce int fallback.
#[test]
fn null_coalesce_int_fallback() {
    ShapeTest::new("None ?? 0").expect_number(0.0);
}

/// Null coalesce negative fallback.
#[test]
fn null_coalesce_negative_fallback() {
    ShapeTest::new("None ?? -1").expect_number(-1.0);
}

// =============================================================================
// SECTION 33: Null comparison with various types
// =============================================================================

/// Int not eq null.
#[test]
fn int_not_eq_null() {
    ShapeTest::new("42 == None").expect_bool(false);
}

/// String not eq null.
#[test]
fn string_not_eq_null() {
    ShapeTest::new(r#""hello" == None"#).expect_bool(false);
}

/// Bool not eq null.
#[test]
fn bool_not_eq_null() {
    ShapeTest::new("true == None").expect_bool(false);
}

/// Float not eq null.
#[test]
fn float_not_eq_null() {
    ShapeTest::new("3.14 == None").expect_bool(false);
}

// =============================================================================
// SECTION 35: Null coalesce in various positions
// =============================================================================

/// Null coalesce as function return.
#[test]
fn null_coalesce_as_function_return() {
    ShapeTest::new("fn get(x: int) { if x > 0 { return x }\nreturn None }\nfn test() -> int { get(-1) ?? 42 }\ntest()")
        .expect_number(42.0);
}

/// Null coalesce as argument.
#[test]
fn null_coalesce_as_argument() {
    ShapeTest::new(
        "fn double(x: int) -> int { x * 2 }\nfn test() -> int { double(None ?? 5) }\ntest()",
    )
    .expect_number(10.0);
}

// =============================================================================
// SECTION 38: Match on nested null coalesce
// =============================================================================

/// Match on coalesced result.
#[test]
fn match_on_coalesced_result() {
    ShapeTest::new("fn test() -> int { let x = None\nlet r = x ?? Ok(42)\nmatch r { Ok(v) => v, Err(e) => -1 } }\ntest()")
        .expect_number(42.0);
}

// =============================================================================
// SECTION 39: Assorted edge cases
// =============================================================================

/// Null coalesce preserves type.
#[test]
fn null_coalesce_preserves_type() {
    ShapeTest::new(r#"None ?? "hello""#).expect_string("hello");
}

/// Null coalesce with null fallback.
#[test]
fn null_coalesce_with_null_fallback() {
    ShapeTest::new("None ?? None").expect_none();
}

// =============================================================================
// SECTION 42: Null coalesce assignment patterns
// =============================================================================

/// Coalesce into variable.
#[test]
fn coalesce_into_variable() {
    ShapeTest::new("let x = None\nlet y = x ?? 42\ny").expect_number(42.0);
}

/// Coalesce chain into variable.
#[test]
fn coalesce_chain_into_variable() {
    ShapeTest::new("let a = None\nlet b = None\nlet c = a ?? b ?? 99\nc").expect_number(99.0);
}

// =============================================================================
// SECTION 43: Various null contexts
// =============================================================================

/// Null in comparison chain.
#[test]
fn null_in_comparison_chain() {
    ShapeTest::new("let x = None\nlet y = None\nx == y").expect_bool(true);
}

/// Null vs non-null neq.
#[test]
fn null_vs_non_null_neq() {
    ShapeTest::new("let x = None\nlet y = 5\nx != y").expect_bool(true);
}

/// Non-null vs null neq.
#[test]
fn non_null_vs_null_neq() {
    ShapeTest::new("let x = 5\nlet y = None\nx != y").expect_bool(true);
}

// =============================================================================
// SECTION 45: Additional edge cases
// =============================================================================

/// Double null coalesce first null.
#[test]
fn double_null_coalesce_first_null() {
    ShapeTest::new("let a = None\nlet b = 5\na ?? b").expect_number(5.0);
}

/// Double null coalesce neither null.
#[test]
fn double_null_coalesce_neither_null() {
    ShapeTest::new("let a = 1\nlet b = 2\na ?? b").expect_number(1.0);
}

/// Null coalesce deeply chained.
#[test]
fn null_coalesce_deeply_chained() {
    ShapeTest::new("None ?? None ?? None ?? None ?? 7").expect_number(7.0);
}

/// Coalesce string chain first present.
#[test]
fn coalesce_string_chain_first_present() {
    ShapeTest::new(r#""first" ?? "second" ?? "third""#).expect_string("first");
}

/// Coalesce string chain first null.
#[test]
fn coalesce_string_chain_first_null() {
    ShapeTest::new(r#"None ?? "second" ?? "third""#).expect_string("second");
}
