//! Stress tests for Option (Some/None), coalesce (??), and None comparison patterns.

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
// SECTION 4: None
// =============================================================================

/// None literal is None.
#[test]
fn none_literal_is_none() {
    ShapeTest::new("None").expect_none();
}

/// None from variable.
#[test]
fn none_from_variable() {
    ShapeTest::new("let x = None\nx").expect_none();
}

/// None equality both None.
#[test]
fn none_equality_both_none() {
    ShapeTest::new("None == None").expect_bool(true);
}

/// None not equal to int.
#[test]
fn none_not_equal_to_int() {
    ShapeTest::new("None != 1").expect_bool(true);
}

/// None not equal to zero.
#[test]
fn none_not_equal_to_zero() {
    ShapeTest::new("None != 0").expect_bool(true);
}

/// None not equal to false.
#[test]
fn none_not_equal_to_false() {
    ShapeTest::new("None != false").expect_bool(true);
}

/// None not equal to string.
#[test]
fn none_not_equal_to_string() {
    ShapeTest::new(r#"None != "hello""#).expect_bool(true);
}

/// None eq int is false.
#[test]
fn none_eq_int_is_false() {
    ShapeTest::new("None == 1").expect_bool(false);
}

/// None eq false is false.
#[test]
fn none_eq_false_is_false() {
    ShapeTest::new("None == false").expect_bool(false);
}

/// None eq empty string is false.
#[test]
fn none_eq_empty_string_is_false() {
    ShapeTest::new(r#"None == """#).expect_bool(false);
}

// =============================================================================
// SECTION 5: Coalesce (??)
// =============================================================================

/// Coalesce None gives fallback.
#[test]
fn none_coalesce_none_gives_fallback() {
    ShapeTest::new("None ?? 42").expect_number(42.0);
}

/// Coalesce value gives value.
#[test]
fn none_coalesce_value_gives_value() {
    ShapeTest::new("10 ?? 42").expect_number(10.0);
}

/// Coalesce zero gives zero.
#[test]
fn none_coalesce_zero_gives_zero() {
    ShapeTest::new("0 ?? 42").expect_number(0.0);
}

/// Coalesce false gives false.
#[test]
fn none_coalesce_false_gives_false() {
    ShapeTest::new("false ?? true").expect_bool(false);
}

/// Coalesce string value.
#[test]
fn none_coalesce_string_value() {
    ShapeTest::new(r#""hi" ?? "fallback""#).expect_string("hi");
}

/// Coalesce None to string fallback.
#[test]
fn none_coalesce_none_to_string_fallback() {
    ShapeTest::new(r#"None ?? "default""#).expect_string("default");
}

/// Coalesce with variable.
#[test]
fn none_coalesce_with_variable() {
    ShapeTest::new("let x = None\nx ?? 99").expect_number(99.0);
}

/// Coalesce with non-None variable.
#[test]
fn none_coalesce_with_non_none_variable() {
    ShapeTest::new("let x = 7\nx ?? 99").expect_number(7.0);
}

// =============================================================================
// SECTION 6: Chained coalesce (??)
// =============================================================================

/// Chained coalesce first None second None.
#[test]
fn chained_coalesce_first_none_second_none() {
    ShapeTest::new("None ?? None ?? 100").expect_number(100.0);
}

/// Chained coalesce first None second value.
#[test]
fn chained_coalesce_first_none_second_value() {
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
    ShapeTest::new(
        "let a: Option<int> = None\nlet b: Option<int> = None\nlet c = 77\na ?? (b ?? c)",
    )
    .expect_number(77.0);
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

/// Match None fallback.
#[test]
fn match_none_fallback() {
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

/// Match none string fallback.
#[test]
fn match_none_string_fallback() {
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

/// Function returning none.
#[test]
fn fn_returning_none() {
    ShapeTest::new("fn test() -> int { let x = None\nx ?? 99 }\ntest()").expect_number(99.0);
}

/// Function conditional some or none — some path.
#[test]
fn fn_conditional_some_or_none() {
    ShapeTest::new(
        "fn maybe(flag: bool) { if flag { return 42 }\nreturn None }\nfn test() -> int { let x = maybe(true)\nx ?? 0 }\ntest()",
    )
    .expect_number(42.0);
}

/// Function conditional None path.
#[test]
fn fn_conditional_none_path() {
    ShapeTest::new(
        "fn maybe(flag: bool) { if flag { return 42 }\nreturn None }\nfn test() -> int { let x = maybe(false)\nx ?? 0 }\ntest()",
    )
    .expect_number(0.0);
}

// =============================================================================
// SECTION 13: Default values with ?? in functions
// =============================================================================

/// Default value pattern None param.
#[test]
fn default_value_pattern_none_param() {
    ShapeTest::new("fn test() -> int { let x = None\nlet val = x ?? 10\nval }\ntest()")
        .expect_number(10.0);
}

/// Default value pattern non-None param.
#[test]
fn default_value_pattern_non_none_param() {
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

/// Option not None check with value.
#[test]
fn option_not_none_check_with_value() {
    ShapeTest::new("fn test() -> bool { let x = 42\nx != None }\ntest()").expect_bool(true);
}

/// Option not None check with None.
#[test]
fn option_not_none_check_with_none() {
    ShapeTest::new("fn test() -> bool { let x = None\nx != None }\ntest()").expect_bool(false);
}

/// Option eq None check with None.
#[test]
fn option_eq_none_check_with_none() {
    ShapeTest::new("fn test() -> bool { let x = None\nx == None }\ntest()").expect_bool(true);
}

/// Option eq None check with value.
#[test]
fn option_eq_none_check_with_value() {
    ShapeTest::new("fn test() -> bool { let x = 42\nx == None }\ntest()").expect_bool(false);
}

/// If not None then use value.
#[test]
fn if_not_none_then_use_value() {
    ShapeTest::new("fn test() -> int { let x = 10\nif x != None { x } else { 0 } }\ntest()")
        .expect_number(10.0);
}

/// If None then default.
#[test]
fn if_none_then_default() {
    ShapeTest::new("fn test() -> int { let x = None\nif x != None { 999 } else { 0 } }\ntest()")
        .expect_number(0.0);
}

// =============================================================================
// SECTION 19: Coalesce with expressions
// =============================================================================

/// Coalesce with arithmetic fallback.
#[test]
fn none_coalesce_with_arithmetic_fallback() {
    ShapeTest::new("None ?? (2 + 3)").expect_number(5.0);
}

/// Coalesce with arithmetic lhs.
#[test]
fn none_coalesce_with_arithmetic_lhs() {
    ShapeTest::new("(1 + 2) ?? 99").expect_number(3.0);
}

/// Coalesce in let binding.
#[test]
fn none_coalesce_in_let_binding() {
    ShapeTest::new("let a = None\nlet b = a ?? 42\nb").expect_number(42.0);
}

// =============================================================================
// SECTION 22: None in various contexts
// =============================================================================

/// None assigned to variable.
#[test]
fn none_assigned_to_variable() {
    ShapeTest::new("let x = None\nx").expect_none();
}

/// None reassigned. Strict-flip: `x` is inferred `int` from its initializer, so
/// reassigning `None` (an `Option`) is a type error — no loose let-mut retype.
#[test]
fn none_reassigned() {
    ShapeTest::new("let mut x = 42\nx = None\nx")
        .expect_run_err_contains("is not compatible with int");
}

/// Variable starts none then assigned. Strict-flip: `x` is inferred `Option`
/// from `None`, so reassigning `10` (an `int`) is a type error.
#[test]
fn variable_starts_none_then_assigned() {
    ShapeTest::new("let mut x = None\nx = 10\nx")
        .expect_run_err_contains("int is not compatible with");
}

/// None in array.
#[test]
fn none_in_array() {
    ShapeTest::new("let arr: Array<Option<int>> = [Some(1), None, Some(3)]\narr[1]")
        .expect_run_err_contains("cannot infer the element type of this array literal");
}

// =============================================================================
// SECTION 24: Coalesce with function calls
// =============================================================================

/// Coalesce with fn returning none.
#[test]
fn none_coalesce_with_fn_returning_none() {
    ShapeTest::new("fn get_val() { return None }\nfn test() -> int { get_val() ?? 42 }\ntest()")
        .expect_number(42.0);
}

/// Coalesce with fn returning value.
#[test]
fn none_coalesce_with_fn_returning_value() {
    ShapeTest::new(
        "fn get_val() -> int { return 10 }\nfn test() -> int { get_val() ?? 42 }\ntest()",
    )
    .expect_number(10.0);
}

// =============================================================================
// SECTION 27: Mixed Ok/Some/none patterns
// =============================================================================

/// Ok inside coalesce.
#[test]
fn ok_inside_none_coalesce() {
    ShapeTest::new("match (Ok(42) ?? 0) { Ok(v) => v, Err(e) => -1 }")
        .expect_run_err_contains("is not compatible with");
}

/// Coalesce then match.
#[test]
fn none_coalesce_then_match() {
    ShapeTest::new("fn test() -> int { let x = None\nlet y = x ?? Ok(42)\nmatch y { Ok(v) => v, Err(e) => -1 } }\ntest()")
        .expect_number(42.0);
}

// =============================================================================
// SECTION 28: Edge cases
// =============================================================================

/// Ok wrapping none.
#[test]
fn ok_wrapping_none() {
    ShapeTest::new("match Ok(None) { Ok(v) => v ?? 99, Err(e) => -1 }").expect_number(99.0);
}

/// Match ok none inner.
#[test]
fn match_ok_none_inner() {
    ShapeTest::new("let x = Ok(None)\nmatch x { Ok(v) => v ?? 99, Err(e) => -1 }")
        .expect_number(99.0);
}

/// Coalesce on false does not trigger.
#[test]
fn none_coalesce_on_false_does_not_trigger() {
    ShapeTest::new("false ?? true").expect_bool(false);
}

/// Coalesce on zero does not trigger.
#[test]
fn none_coalesce_on_zero_does_not_trigger() {
    ShapeTest::new("0 ?? 999").expect_number(0.0);
}

/// Coalesce on empty string does not trigger.
#[test]
fn none_coalesce_on_empty_string_does_not_trigger() {
    ShapeTest::new(r#""" ?? "fallback""#).expect_string("");
}

// =============================================================================
// SECTION 30: Coalesce with different types
// =============================================================================

/// Coalesce bool fallback.
#[test]
fn none_coalesce_bool_fallback() {
    ShapeTest::new("None ?? true").expect_bool(true);
}

/// Coalesce int fallback.
#[test]
fn none_coalesce_int_fallback() {
    ShapeTest::new("None ?? 0").expect_number(0.0);
}

/// Coalesce negative fallback.
#[test]
fn none_coalesce_negative_fallback() {
    ShapeTest::new("None ?? -1").expect_number(-1.0);
}

// =============================================================================
// SECTION 33: None comparison with various types
// =============================================================================

/// Int not eq None.
#[test]
fn int_not_eq_none() {
    ShapeTest::new("42 == None").expect_bool(false);
}

/// String not eq None.
#[test]
fn string_not_eq_none() {
    ShapeTest::new(r#""hello" == None"#).expect_bool(false);
}

/// Bool not eq None.
#[test]
fn bool_not_eq_none() {
    ShapeTest::new("true == None").expect_bool(false);
}

/// Float not eq None.
#[test]
fn float_not_eq_none() {
    ShapeTest::new("3.14 == None").expect_bool(false);
}

// =============================================================================
// SECTION 35: Coalesce in various positions
// =============================================================================

/// Coalesce as function return.
#[test]
fn none_coalesce_as_function_return() {
    ShapeTest::new("fn get(x: int) { if x > 0 { return x }\nreturn None }\nfn test() -> int { get(-1) ?? 42 }\ntest()")
        .expect_number(42.0);
}

/// Coalesce as argument.
#[test]
fn none_coalesce_as_argument() {
    ShapeTest::new(
        "fn double(x: int) -> int { x * 2 }\nfn test() -> int { double(None ?? 5) }\ntest()",
    )
    .expect_number(10.0);
}

// =============================================================================
// SECTION 38: Match on nested coalesce
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

/// Coalesce preserves type.
#[test]
fn none_coalesce_preserves_type() {
    ShapeTest::new(r#"None ?? "hello""#).expect_string("hello");
}

/// Coalesce with None fallback.
#[test]
fn none_coalesce_with_none_fallback() {
    ShapeTest::new("None ?? None").expect_none();
}

// =============================================================================
// SECTION 42: Coalesce assignment patterns
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
// SECTION 43: Various None contexts
// =============================================================================

/// None in comparison chain.
#[test]
fn none_in_comparison_chain() {
    ShapeTest::new("let x = None\nlet y = None\nx == y")
        .expect_run_err_contains("Cannot infer types for binary operation `Equal`");
}

/// None vs non-None neq.
#[test]
fn none_vs_non_none_neq() {
    ShapeTest::new("let x = None\nlet y = 5\nx != y").expect_bool(true);
}

/// Non-None vs None neq.
#[test]
fn non_none_vs_none_neq() {
    ShapeTest::new("let x = 5\nlet y = None\nx != y").expect_bool(true);
}

// =============================================================================
// SECTION 45: Additional edge cases
// =============================================================================

/// Double coalesce first None.
#[test]
fn double_none_coalesce_first_none() {
    ShapeTest::new("let a = None\nlet b = 5\na ?? b").expect_number(5.0);
}

/// Double coalesce neither None.
#[test]
fn double_none_coalesce_neither_none() {
    ShapeTest::new("let a = 1\nlet b = 2\na ?? b").expect_number(1.0);
}

/// Coalesce deeply chained.
#[test]
fn none_coalesce_deeply_chained() {
    ShapeTest::new("None ?? None ?? None ?? None ?? 7").expect_number(7.0);
}

/// Coalesce string chain first present.
#[test]
fn coalesce_string_chain_first_present() {
    ShapeTest::new(r#""first" ?? "second" ?? "third""#).expect_string("first");
}

/// Coalesce string chain first None.
#[test]
fn coalesce_string_chain_first_none() {
    ShapeTest::new(r#"None ?? "second" ?? "third""#).expect_string("second");
}
