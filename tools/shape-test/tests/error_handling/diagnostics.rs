//! Diagnostic tests for error handling in Shape.
//!
//! Covers: parsing validation (syntax acceptance), semantic diagnostics
//! (no spurious errors, known inference bugs), parse error quality,
//! and runtime error quality.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Parsing - ensure syntax is accepted
// =========================================================================

#[test]
fn ok_constructor_parses() {
    ShapeTest::new("let x = Ok(42)").expect_parse_ok();
}

#[test]
fn err_constructor_parses() {
    ShapeTest::new("let x = Err(\"fail\")").expect_parse_ok();
}

#[test]
fn some_constructor_parses() {
    ShapeTest::new("let x = Some(7)").expect_parse_ok();
}

#[test]
fn none_literal_parses() {
    ShapeTest::new("let x = None").expect_parse_ok();
}

#[test]
fn try_operator_parses() {
    let code = r#"
fn f() -> Result<int> {
  let x = Ok(1)?
  Ok(x)
}
"#;
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn context_operator_parses() {
    ShapeTest::new("let x = (Err(\"a\") !! \"b\")").expect_parse_ok();
}

#[test]
fn coalesce_operator_parses() {
    ShapeTest::new("let x = None ?? 42").expect_parse_ok();
}

#[test]
fn result_return_type_annotation_parses() {
    let code = r#"
fn test() -> Result<int> {
  Ok(42)
}
"#;
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn option_type_annotation_parses() {
    ShapeTest::new("let x: Option<int> = Some(42)").expect_parse_ok();
}

#[test]
fn fallible_type_assertion_parses() {
    ShapeTest::new("let x = \"42\" as int?").expect_parse_ok();
}

// =========================================================================
// Semantic diagnostics - no spurious errors
// =========================================================================

#[test]
fn ok_err_no_semantic_diagnostics() {
    let code = r#"
let a = Ok(42)
let b = Err("fail")
let c = Some(7)
let d = None
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn try_operator_in_result_fn_if_else_inference_bug() {
    // Regression: explicit Result<T> return annotations must constrain
    // Err(...) branches without spurious generic inference diagnostics.
    let code = r#"
fn might_fail(x: int) -> Result<int> {
  if x < 0 { return Err("negative") }
  return Ok(x * 2)
}

fn process() -> Result<int> {
  let val = might_fail(5)?
  return Ok(val + 1)
}
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn context_operator_err_only_produces_generic_inference_diagnostic() {
    // Regression: context + try around Err(...) should be constrained by
    // explicit Result<T> return annotation.
    let code = r#"
fn test() -> Result<int> {
  let x = (Err("fail") !! "context")?
  return Ok(x)
}
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn coalesce_operator_no_diagnostics() {
    let code = r#"
let maybe: Option<int> = None
let val = maybe ?? 0
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn fallible_type_assertion_no_semantic_diagnostics_for_supported_conversion() {
    let code = r#"
impl TryInto<int> for string as int {
  method tryInto() {
    __try_into_int(self)
  }
}

fn parse_int(raw: string) -> Result<int> {
  let n = (raw as int?)?
  return Ok(n)
}
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn fallible_type_assertion_no_semantic_diagnostics_for_named_try_into_impl() {
    let code = r#"
impl TryInto<int> for string as int {
  method tryInto() {
    Ok(42)
  }
}

fn parse_price(p: string) -> Result<int> {
  let n = (p as int?)?
  return Ok(n)
}
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn infallible_type_assertion_no_semantic_diagnostics_for_supported_into_conversion() {
    let code = r#"
impl Into<int> for bool as int {
  method into() {
    __into_int(self)
  }
}

let n = true as int
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn infallible_type_assertion_reports_static_conversion_error() {
    let code = r#"
let bad = "42" as int
"#;
    ShapeTest::new(code).expect_semantic_diagnostic_contains("Cannot assert type");
}

#[test]
fn fallible_type_assertion_reports_static_conversion_error() {
    let code = r#"
let bad = { x: 1 } as int?
"#;
    ShapeTest::new(code).expect_semantic_diagnostic_contains("Cannot assert type");
}

#[test]
fn try_operator_reports_non_fallible_operand_error() {
    let code = r#"
fn run() -> Result<int> {
  let x = 42?
  Ok(x)
}
"#;
    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("try operator '?' expects Result<T, E> or Option<T>");
}

// =========================================================================
// Compile-time `as` cast validation and Option/Result lifting
// =========================================================================

// -- Compile errors for invalid infallible casts --

#[test]
fn infallible_cast_string_to_int_is_rejected() {
    // string has TryInto<int> but NOT Into<int>, so `as int` must fail
    let code = r#"
let bad = "42" as int
"#;
    ShapeTest::new(code).expect_semantic_diagnostic_contains("Cannot assert type");
}

// -- Valid direct conversions (no semantic diagnostics) --

#[test]
fn infallible_cast_int_to_number_no_semantic_diagnostics() {
    let code = r#"
impl Into<number> for int as number {
  method into() { 0.0 }
}
let n = 42 as number
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn infallible_cast_bool_to_int_no_semantic_diagnostics() {
    let code = r#"
impl Into<int> for bool as int {
  method into() { 0 }
}
let n = true as int
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn fallible_cast_string_to_int_no_semantic_diagnostics() {
    let code = r#"
impl TryInto<int> for string as int {
  method tryInto() { Ok(0) }
}
let r = "42" as int?
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

// -- Width casts are unaffected by Into validation --

#[test]
fn width_cast_i8_always_valid() {
    ShapeTest::new("let x = 256 as i8\nx").expect_run_ok();
}

// -- Option/Result lifting: type inference accepts these casts --

#[test]
fn option_int_as_number_no_semantic_diagnostics() {
    let code = r#"
impl Into<number> for int as number {
  method into() { 0.0 }
}
let opt: Option<int> = Some(42)
let val = opt as number
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

#[test]
fn result_int_as_number_no_semantic_diagnostics() {
    let code = r#"
impl Into<number> for int as number {
  method into() { 0.0 }
}
let res: Result<int> = Ok(42)
let val = res as number
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

// =========================================================================
// Parse error quality (from programs_error_handling.rs)
// =========================================================================

#[test]
fn parse_err_missing_closing_brace() {
    ShapeTest::new(
        r#"
        fn foo() {
            let x = 1
    "#,
    )
    .expect_parse_err();
}

#[test]
fn parse_err_missing_closing_paren() {
    ShapeTest::new(
        r#"
        let x = (1 + 2
    "#,
    )
    .expect_parse_err();
}

#[test]
fn parse_err_invalid_let() {
    ShapeTest::new("let = ;").expect_parse_err();
}

#[test]
fn parse_err_unterminated_string() {
    ShapeTest::new(
        r#"
        let s = "hello
    "#,
    )
    .expect_parse_err();
}

#[test]
fn parse_err_empty_program_ok() {
    ShapeTest::new("").expect_parse_ok();
}

#[test]
fn parse_err_lone_operator() {
    ShapeTest::new("+").expect_parse_err();
}

#[test]
fn parse_err_double_operator() {
    ShapeTest::new("1 + + 2").expect_parse_err();
}

#[test]
fn parse_err_missing_match_arrow() {
    ShapeTest::new(
        r#"
        match 1 {
            1 "one"
        }
    "#,
    )
    .expect_parse_err();
}

#[test]
fn parse_err_unclosed_bracket() {
    ShapeTest::new(
        r#"
        let a = [1, 2, 3
    "#,
    )
    .expect_parse_err();
}

// fn without body followed by let parses as declaration-only fn (valid syntax).
#[test]
fn parse_err_missing_fn_body_is_declaration() {
    ShapeTest::new(
        r#"
        fn foo()
        let x = 1
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn parse_err_unmatched_parens_nested() {
    ShapeTest::new("((1 + 2)").expect_parse_err();
}

// =========================================================================
// Runtime error quality (from programs_error_handling.rs)
// =========================================================================

#[test]
fn runtime_err_division_by_zero() {
    ShapeTest::new(
        r#"
        1 / 0
    "#,
    )
    .expect_run_err();
}

// Array out-of-bounds returns null in Shape (not an error).
#[test]
fn runtime_err_array_index_out_of_bounds_returns_null() {
    ShapeTest::new(
        r#"
        let a = [1, 2, 3]
        let v = a[10]
        v == None
    "#,
    )
    .expect_bool(true);
}

// Negative out-of-bounds also returns null.
#[test]
fn runtime_err_negative_index_beyond_length_returns_null() {
    ShapeTest::new(
        r#"
        let a = [1, 2, 3]
        let v = a[-10]
        v == None
    "#,
    )
    .expect_bool(true);
}

#[test]
fn runtime_err_stack_overflow() {
    ShapeTest::new(
        r#"
        fn infinite() { infinite() }
        infinite()
    "#,
    )
    .expect_run_err();
}

#[test]
fn runtime_err_type_error_subtract() {
    ShapeTest::new(
        r#"
        "hello" - 1
    "#,
    )
    .expect_run_err();
}

#[test]
fn runtime_err_undefined_variable() {
    ShapeTest::new(
        r#"
        x + 1
    "#,
    )
    .expect_run_err();
}

#[test]
fn runtime_err_call_non_function() {
    ShapeTest::new(
        r#"
        let x = 42
        x()
    "#,
    )
    .expect_run_err();
}

#[test]
fn runtime_err_modulo_by_zero() {
    ShapeTest::new(
        r#"
        10 % 0
    "#,
    )
    .expect_run_err();
}

// Empty array access returns null (not an error).
#[test]
fn runtime_err_empty_array_access_returns_null() {
    ShapeTest::new(
        r#"
        let a = []
        let v = a[0]
        v == None
    "#,
    )
    .expect_bool(true);
}

#[test]
fn runtime_err_wrong_arg_count_too_few() {
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        add(1)
    "#,
    )
    .expect_run_err();
}

#[test]
fn runtime_err_wrong_arg_count_too_many() {
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        add(1, 2, 3)
    "#,
    )
    .expect_run_err();
}

#[test]
fn runtime_err_division_by_zero_message() {
    ShapeTest::new(
        r#"
        1 / 0
    "#,
    )
    .expect_run_err_contains("ivision");
}
