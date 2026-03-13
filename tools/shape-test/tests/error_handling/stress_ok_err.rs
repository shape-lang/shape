//! Stress tests for Ok/Err creation, Result matching, wrapping, and identity checks.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 1: Ok creation
// =============================================================================

/// Ok wraps int value, extract via match.
#[test]
fn ok_wrap_int() {
    ShapeTest::new("match Ok(42) { Ok(v) => v, Err(e) => -1 }").expect_number(42.0);
}

/// Ok wraps zero.
#[test]
fn ok_wrap_zero() {
    ShapeTest::new("match Ok(0) { Ok(v) => v, Err(e) => -1 }").expect_number(0.0);
}

/// Ok wraps negative.
#[test]
fn ok_wrap_negative() {
    ShapeTest::new("match Ok(-1) { Ok(v) => v, Err(e) => 999 }").expect_number(-1.0);
}

/// Ok wraps bool true.
#[test]
fn ok_wrap_bool_true() {
    ShapeTest::new("match Ok(true) { Ok(v) => v, Err(e) => false }").expect_bool(true);
}

/// Ok wraps bool false.
#[test]
fn ok_wrap_bool_false() {
    ShapeTest::new("match Ok(false) { Ok(v) => v, Err(e) => true }").expect_bool(false);
}

/// Ok wraps string.
#[test]
fn ok_wrap_string() {
    ShapeTest::new(r#"match Ok("hello") { Ok(v) => v, Err(e) => "err" }"#).expect_string("hello");
}

/// Ok wraps float.
#[test]
fn ok_wrap_float() {
    ShapeTest::new("match Ok(3.14) { Ok(v) => v, Err(e) => 0.0 }").expect_number(3.14);
}

/// Ok wraps large int.
#[test]
fn ok_wrap_large_int() {
    ShapeTest::new("match Ok(999999) { Ok(v) => v, Err(e) => -1 }").expect_number(999999.0);
}

// =============================================================================
// SECTION 2: Err creation
// =============================================================================

/// Err wraps string, match detects error.
#[test]
fn err_wrap_string() {
    ShapeTest::new(r#"match Err("something went wrong") { Ok(v) => 1, Err(e) => -1 }"#)
        .expect_number(-1.0);
}

/// Err wraps short string.
#[test]
fn err_wrap_short_string() {
    ShapeTest::new(r#"match Err("fail") { Ok(v) => 1, Err(e) => -1 }"#).expect_number(-1.0);
}

/// Err wraps empty string.
#[test]
fn err_wrap_empty_string() {
    ShapeTest::new(r#"match Err("") { Ok(v) => 1, Err(e) => -1 }"#).expect_number(-1.0);
}

/// Err wraps int payload.
#[test]
fn err_wrap_int() {
    ShapeTest::new("match Err(404) { Ok(v) => 1, Err(e) => -1 }").expect_number(-1.0);
}

/// Err is not Ok.
#[test]
fn err_is_not_ok() {
    ShapeTest::new(r#"match Err("bad") { Ok(v) => 1, Err(e) => -1 }"#).expect_number(-1.0);
}

/// Ok is not Err.
#[test]
fn ok_is_not_err() {
    ShapeTest::new("match Ok(1) { Ok(v) => v, Err(e) => -1 }").expect_number(1.0);
}

// =============================================================================
// SECTION 7: Match on Result (Ok/Err arms)
// =============================================================================

/// Match Ok extracts value.
#[test]
fn match_ok_extracts_value() {
    ShapeTest::new("let x = Ok(42)\nmatch x { Ok(v) => v, Err(e) => -1 }").expect_number(42.0);
}

/// Match Err extracts error.
#[test]
fn match_err_extracts_error() {
    ShapeTest::new(
        r#"let x = Err("fail")
match x { Ok(v) => 0, Err(e) => -1 }"#,
    )
    .expect_number(-1.0);
}

/// Match Ok with string payload.
#[test]
fn match_ok_with_string_payload() {
    ShapeTest::new(
        r#"let x = Ok("success")
match x { Ok(v) => v, Err(e) => "failed" }"#,
    )
    .expect_string("success");
}

/// Match Err with string message.
#[test]
fn match_err_with_string_message() {
    ShapeTest::new(
        r#"let x = Err("boom")
match x { Ok(v) => "ok", Err(e) => e }"#,
    )
    .expect_string("boom");
}

/// Match Ok with bool payload.
#[test]
fn match_ok_with_bool_payload() {
    ShapeTest::new("let x = Ok(true)\nmatch x { Ok(v) => v, Err(e) => false }").expect_bool(true);
}

/// Match Ok zero value.
#[test]
fn match_ok_zero_value() {
    ShapeTest::new("let x = Ok(0)\nmatch x { Ok(v) => v, Err(e) => -1 }").expect_number(0.0);
}

/// Match Err returns fallback int.
#[test]
fn match_err_returns_fallback_int() {
    ShapeTest::new(
        r#"let x = Err("nope")
match x { Ok(v) => 100, Err(e) => 200 }"#,
    )
    .expect_number(200.0);
}

// =============================================================================
// SECTION 11: Ok/Err wrapping compound values
// =============================================================================

/// Ok wraps array.
#[test]
fn ok_wrap_array() {
    ShapeTest::new("match Ok([1, 2, 3]) { Ok(v) => v.length, Err(e) => -1 }").expect_number(3.0);
}

/// Err wrap with number payload.
#[test]
fn err_wrap_with_number_payload() {
    ShapeTest::new("match Err(42) { Ok(v) => 1, Err(e) => -1 }").expect_number(-1.0);
}

// =============================================================================
// SECTION 12: Nested Result
// =============================================================================

/// Nested Ok(Ok(42)).
#[test]
fn nested_result_ok_ok() {
    ShapeTest::new(
        "match Ok(Ok(42)) { Ok(inner) => match inner { Ok(v) => v, Err(e) => -1 }, Err(e) => -2 }",
    )
    .expect_number(42.0);
}

/// Nested Ok(Err(...)).
#[test]
fn nested_result_ok_err() {
    ShapeTest::new(r#"match Ok(Err("inner fail")) { Ok(inner) => match inner { Ok(v) => 1, Err(e) => -1 }, Err(e) => -2 }"#)
        .expect_number(-1.0);
}

/// Nested Err wrapper.
#[test]
fn nested_result_err_wrapper() {
    ShapeTest::new(r#"match Err("outer fail") { Ok(v) => 1, Err(e) => -1 }"#).expect_number(-1.0);
}

// =============================================================================
// SECTION 20: Match on Result with computation
// =============================================================================

/// Match Ok with computation in arm.
#[test]
fn match_ok_with_computation_in_arm() {
    ShapeTest::new("let x = Ok(10)\nmatch x { Ok(v) => v + 5, Err(e) => -1 }").expect_number(15.0);
}

/// Match Err with fallback computation.
#[test]
fn match_err_with_fallback_computation() {
    ShapeTest::new(
        r#"let x = Err("bad")
match x { Ok(v) => v, Err(e) => 100 + 200 }"#,
    )
    .expect_number(300.0);
}

/// Match Ok with multiply.
#[test]
fn match_ok_with_multiply() {
    ShapeTest::new("let x = Ok(7)\nmatch x { Ok(v) => v * 3, Err(e) => 0 }").expect_number(21.0);
}

// =============================================================================
// SECTION 23: Ok/Err with match wildcard
// =============================================================================

/// Match result with wildcard err.
#[test]
fn match_result_with_wildcard_err() {
    ShapeTest::new(
        r#"let x = Err("fail")
match x { Ok(v) => v, Err(_) => -1 }"#,
    )
    .expect_number(-1.0);
}

/// Match result with wildcard ok.
#[test]
fn match_result_with_wildcard_ok() {
    ShapeTest::new("let x = Ok(42)\nmatch x { Ok(_) => 1, Err(_) => -1 }").expect_number(1.0);
}

// =============================================================================
// SECTION 25: Result used as a value (not matched)
// =============================================================================

/// Ok stored in variable.
#[test]
fn ok_stored_in_variable() {
    ShapeTest::new("let r = Ok(42)\nmatch r { Ok(v) => v, Err(e) => -1 }").expect_number(42.0);
}

/// Err stored in variable.
#[test]
fn err_stored_in_variable() {
    ShapeTest::new(
        r#"let r = Err("fail")
match r { Ok(v) => 1, Err(e) => -1 }"#,
    )
    .expect_number(-1.0);
}

/// Ok reassigned to Err.
#[test]
fn ok_reassigned_to_err() {
    ShapeTest::new(
        r#"let mut r = Ok(1)
r = Err("changed")
match r { Ok(v) => v, Err(e) => -1 }"#,
    )
    .expect_number(-1.0);
}

/// Err reassigned to Ok.
#[test]
fn err_reassigned_to_ok() {
    ShapeTest::new(
        r#"let mut r = Err("fail")
r = Ok(42)
match r { Ok(v) => v, Err(e) => -1 }"#,
    )
    .expect_number(42.0);
}

// =============================================================================
// SECTION 26: Chained match on Results
// =============================================================================

/// Chained result match.
#[test]
fn chained_result_match() {
    ShapeTest::new(
        r#"fn step1() { return Ok(10) }
fn step2(x: int) { return Ok(x + 5) }
fn test() -> int {
    let r1 = step1()
    let val = match r1 { Ok(v) => v, Err(e) => -1 }
    let r2 = step2(val)
    match r2 { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(15.0);
}

/// Chained result match first fails.
#[test]
fn chained_result_match_first_fails() {
    ShapeTest::new(
        r#"fn step1() { return Err("fail") }
fn step2(x: int) { return Ok(x + 5) }
fn test() -> int {
    let r1 = step1()
    let val = match r1 { Ok(v) => v, Err(e) => -1 }
    val
}
test()"#,
    )
    .expect_number(-1.0);
}

// =============================================================================
// SECTION 29: Match Result - err string payload
// =============================================================================

/// Match err string payload extraction.
#[test]
fn match_err_string_payload_extraction() {
    ShapeTest::new(
        r#"let x = Err("error message")
match x { Ok(v) => "no error", Err(e) => e }"#,
    )
    .expect_string("error message");
}

/// Match err short payload.
#[test]
fn match_err_short_payload() {
    ShapeTest::new(
        r#"let x = Err("e")
match x { Ok(v) => "ok", Err(e) => e }"#,
    )
    .expect_string("e");
}

// =============================================================================
// SECTION 31: Result inside conditional
// =============================================================================

/// Result in if-then-else true path.
#[test]
fn result_in_if_then_else() {
    ShapeTest::new(
        r#"fn test() -> int {
    let flag = true
    let r = if flag { Ok(10) } else { Err("no") }
    match r { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(10.0);
}

/// Result in if-then-else false path.
#[test]
fn result_in_if_then_else_false() {
    ShapeTest::new(
        r#"fn test() -> int {
    let flag = false
    let r = if flag { Ok(10) } else { Err("no") }
    match r { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(-1.0);
}

// =============================================================================
// SECTION 32: Multiple variables with Result
// =============================================================================

/// Two Ok results matched.
#[test]
fn two_ok_results_matched() {
    ShapeTest::new(
        "fn test() -> int { let a = Ok(10)\nlet b = Ok(20)\nlet va = match a { Ok(v) => v, Err(e) => 0 }\nlet vb = match b { Ok(v) => v, Err(e) => 0 }\nva + vb }\ntest()",
    )
    .expect_number(30.0);
}

/// One Ok one Err results matched.
#[test]
fn one_ok_one_err_results_matched() {
    ShapeTest::new(
        r#"fn test() -> int {
    let a = Ok(10)
    let b = Err("fail")
    let va = match a { Ok(v) => v, Err(e) => 0 }
    let vb = match b { Ok(v) => v, Err(e) => 0 }
    va + vb
}
test()"#,
    )
    .expect_number(10.0);
}

// =============================================================================
// SECTION 34: Complex Ok/Err flows
// =============================================================================

/// Result chain three steps all ok.
#[test]
fn result_chain_three_steps_all_ok() {
    ShapeTest::new(
        r#"fn add_one(x: int) { return Ok(x + 1) }
fn test() -> int {
    let r1 = add_one(0)
    let v1 = match r1 { Ok(v) => v, Err(e) => -1 }
    let r2 = add_one(v1)
    let v2 = match r2 { Ok(v) => v, Err(e) => -1 }
    let r3 = add_one(v2)
    match r3 { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(3.0);
}

/// Ok with computed expression.
#[test]
fn ok_with_computed_expression() {
    ShapeTest::new("match Ok(2 + 3 * 4) { Ok(v) => v, Err(e) => -1 }").expect_number(14.0);
}

/// Err with string concat.
#[test]
fn err_with_string_concat() {
    ShapeTest::new(r#"match Err("error" + " message") { Ok(v) => 1, Err(e) => -1 }"#)
        .expect_number(-1.0);
}

// =============================================================================
// SECTION 37: Ok/Err identity checks
// =============================================================================

/// Ok is not none.
#[test]
fn ok_is_not_none() {
    ShapeTest::new("Ok(1) != None").expect_bool(true);
}

/// Err is not none.
#[test]
fn err_is_not_none() {
    ShapeTest::new(r#"Err("fail") != None"#).expect_bool(true);
}

/// Some is not none.
#[test]
fn some_is_not_none() {
    ShapeTest::new("Some(1) != None").expect_bool(true);
}

/// Null is none.
#[test]
fn null_is_none() {
    ShapeTest::new("None == None").expect_bool(true);
}

// =============================================================================
// SECTION 39: Assorted Result/Option edge cases
// =============================================================================

/// Ok containing negative zero.
#[test]
fn ok_containing_negative_zero() {
    ShapeTest::new("match Ok(-0.0) { Ok(v) => v, Err(e) => 999.0 }").expect_number(0.0);
}

/// Match deeply nested ok.
#[test]
fn match_deeply_nested_ok() {
    ShapeTest::new(
        "let x = Ok(Ok(Ok(42)))\nmatch x { Ok(inner) => match inner { Ok(inner2) => match inner2 { Ok(v) => v, Err(e) => -1 }, Err(e) => -2 }, Err(e) => -3 }",
    )
    .expect_number(42.0);
}

// =============================================================================
// SECTION 40: Compile-time error detection
// =============================================================================

/// Ok without argument fails.
#[test]
fn ok_without_argument_fails() {
    ShapeTest::new("Ok()").expect_run_err();
}

/// Err without argument fails.
#[test]
fn err_without_argument_fails() {
    ShapeTest::new("Err()").expect_run_err();
}

/// Some without argument fails.
#[test]
fn some_without_argument_fails() {
    ShapeTest::new("Some()").expect_run_err();
}

// =============================================================================
// SECTION 41: Functional composition with Result
// =============================================================================

/// Map result with match — safe division.
#[test]
fn map_result_with_match() {
    ShapeTest::new(
        r#"fn safe_div(a: int, b: int) {
    if b == 0 { return Err("div by zero") }
    return Ok(a / b)
}
fn test() -> int {
    let r = safe_div(10, 2)
    match r { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(5.0);
}

/// Safe division by zero returns err.
#[test]
fn safe_div_by_zero_returns_err() {
    ShapeTest::new(
        r#"fn safe_div(a: int, b: int) {
    if b == 0 { return Err("div by zero") }
    return Ok(a / b)
}
fn test() -> int {
    let r = safe_div(10, 0)
    match r { Ok(v) => v, Err(e) => -999 }
}
test()"#,
    )
    .expect_number(-999.0);
}

// =============================================================================
// SECTION 44: More match patterns on Result
// =============================================================================

/// Match result computed ok value.
#[test]
fn match_result_computed_ok_value() {
    ShapeTest::new(
        "fn test() -> int { let x = Ok(3 * 7)\nmatch x { Ok(v) => v, Err(e) => 0 } }\ntest()",
    )
    .expect_number(21.0);
}

/// Match result ok float.
#[test]
fn match_result_ok_float() {
    ShapeTest::new("let x = Ok(2.5)\nmatch x { Ok(v) => v, Err(e) => 0.0 }").expect_number(2.5);
}

/// Match result returns bool.
#[test]
fn match_result_returns_bool() {
    ShapeTest::new("let x = Ok(true)\nmatch x { Ok(v) => v, Err(e) => false }").expect_bool(true);
}

// =============================================================================
// SECTION 45: Additional edge cases
// =============================================================================

/// Match on direct Ok literal.
#[test]
fn match_on_direct_ok_literal() {
    ShapeTest::new("match Ok(99) { Ok(v) => v, Err(e) => -1 }").expect_number(99.0);
}

/// Match on direct Err literal.
#[test]
fn match_on_direct_err_literal() {
    ShapeTest::new(r#"match Err("boom") { Ok(v) => 1, Err(e) => -1 }"#).expect_number(-1.0);
}

/// Ok wrapping large negative.
#[test]
fn ok_wrapping_large_negative() {
    ShapeTest::new("match Ok(-999999) { Ok(v) => v, Err(e) => 0 }").expect_number(-999999.0);
}

/// Err wrap bool payload.
#[test]
fn err_wrap_bool_payload() {
    ShapeTest::new("match Err(false) { Ok(v) => 1, Err(e) => -1 }").expect_number(-1.0);
}

/// Ok wrapping false.
#[test]
fn ok_wrapping_false() {
    ShapeTest::new("match Ok(false) { Ok(v) => v, Err(e) => true }").expect_bool(false);
}

/// Ok wrapping empty string.
#[test]
fn ok_wrapping_empty_string() {
    ShapeTest::new(r#"match Ok("") { Ok(v) => v, Err(e) => "err" }"#).expect_string("");
}
