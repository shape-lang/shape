//! Tests for Result/Option creation, matching, and coalesce operator (??) in Shape.
//!
//! Covers: Ok/Err constructors with various payloads, Some/None constructors,
//! Result matching patterns, Result in data structures, coalesce operator (??),
//! and Option propagation.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Ok and Err constructors (from main.rs)
// =========================================================================

#[test]
fn ok_constructor_wraps_integer() {
    ShapeTest::new("let x = Ok(42)\nprint(x)")
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("42");
}

#[test]
fn err_constructor_with_string_payload() {
    ShapeTest::new("let a = Err(\"disk full\")\nprint(a)")
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn err_constructor_with_integer_payload() {
    ShapeTest::new("let b = Err(404)\nprint(b)")
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn err_constructor_with_object_payload() {
    let code = r#"let c = Err({ code: "IO", path: "/tmp/a.txt" })
print(c)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn ok_constructor_wraps_string() {
    ShapeTest::new("let x = Ok(\"hello\")\nprint(x)")
        .expect_run_ok()
        .expect_output_contains("Ok");
}

#[test]
fn ok_constructor_wraps_bool() {
    ShapeTest::new("let x = Ok(true)\nprint(x)")
        .expect_run_ok()
        .expect_output_contains("Ok");
}

#[test]
fn ok_constructor_wraps_float() {
    ShapeTest::new("let x = Ok(3.14)\nprint(x)")
        .expect_run_ok()
        .expect_output_contains("Ok");
}

// =========================================================================
// Some and None constructors (from main.rs)
// =========================================================================

#[test]
fn some_constructor_wraps_integer() {
    ShapeTest::new("let maybe_id = Some(7)\nprint(maybe_id)")
        .expect_run_ok()
        .expect_output_contains("7");
}

#[test]
fn none_value_prints() {
    ShapeTest::new("let missing = None\nprint(missing)")
        .expect_run_ok()
        .expect_output_contains("None");
}

#[test]
fn some_constructor_wraps_string() {
    ShapeTest::new("let s = Some(\"hello\")\nprint(s)")
        .expect_run_ok()
        .expect_output_contains("hello");
}

// =========================================================================
// Result creation and matching (from programs_error_handling.rs)
// =========================================================================

#[test]
fn result_ok_number() {
    ShapeTest::new(
        r#"
        let r = Ok(42)
        match r {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn result_ok_string() {
    ShapeTest::new(
        r#"
        let r = Ok("hello")
        match r {
            Ok(v) => v
            Err(_) => "error"
        }
    "#,
    )
    .expect_string("hello");
}

#[test]
fn result_ok_bool() {
    ShapeTest::new(
        r#"
        let r = Ok(true)
        match r {
            Ok(v) => v
            Err(_) => false
        }
    "#,
    )
    .expect_bool(true);
}

#[test]
fn result_err_string() {
    ShapeTest::new(
        r#"
        let r = Err("fail")
        match r {
            Ok(v) => "ok"
            Err(_) => "got error"
        }
    "#,
    )
    .expect_string("got error");
}

#[test]
fn result_err_number() {
    ShapeTest::new(
        r#"
        let r = Err(404)
        match r {
            Ok(v) => 0
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn result_match_ok_extracts_value() {
    ShapeTest::new(
        r#"
        let r = Ok(100)
        match r {
            Ok(v) => v + 1
            Err(_) => 0
        }
    "#,
    )
    .expect_number(101.0);
}

#[test]
fn result_match_err_catches() {
    ShapeTest::new(
        r#"
        let r = Err("oops")
        match r {
            Ok(v) => "ok"
            Err(e) => "caught"
        }
    "#,
    )
    .expect_string("caught");
}

#[test]
fn result_as_function_return_ok() {
    ShapeTest::new(
        r#"
        fn compute(x) -> Result<number> {
            if x > 0 { Ok(x * 2) }
            else { Err("negative input") }
        }
        match compute(5) {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn result_as_function_return_err() {
    ShapeTest::new(
        r#"
        fn compute(x) -> Result<number> {
            if x > 0 { Ok(x * 2) }
            else { Err("negative input") }
        }
        match compute(-1) {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn result_in_variable_binding() {
    ShapeTest::new(
        r#"
        let r = Ok(7)
        let val = match r {
            Ok(v) => v
            Err(_) => 0
        }
        val * 3
    "#,
    )
    .expect_number(21.0);
}

#[test]
fn result_in_array() {
    ShapeTest::new(
        r#"
        let results = [Ok(1), Ok(2), Err("skip"), Ok(4)]
        let mut sum = 0
        for r in results {
            match r {
                Ok(v) => { sum = sum + v }
                Err(_) => {}
            }
        }
        sum
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn result_ok_with_zero() {
    ShapeTest::new(
        r#"
        let r = Ok(0)
        match r {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn result_ok_with_false() {
    ShapeTest::new(
        r#"
        let r = Ok(false)
        match r {
            Ok(v) => v
            Err(_) => true
        }
    "#,
    )
    .expect_bool(false);
}

#[test]
fn result_ok_with_empty_string() {
    ShapeTest::new(
        r#"
        let r = Ok("")
        match r {
            Ok(v) => "got ok"
            Err(_) => "got err"
        }
    "#,
    )
    .expect_string("got ok");
}

#[test]
fn result_err_with_empty_string() {
    ShapeTest::new(
        r#"
        let r = Err("")
        match r {
            Ok(v) => "ok"
            Err(_) => "err"
        }
    "#,
    )
    .expect_string("err");
}

#[test]
fn result_nested_ok_in_ok() {
    ShapeTest::new(
        r#"
        let r = Ok(Ok(42))
        match r {
            Ok(inner) => {
                match inner {
                    Ok(v) => v
                    Err(_) => -1
                }
            }
            Err(_) => -2
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn result_nested_err_in_ok() {
    ShapeTest::new(
        r#"
        let r = Ok(Err("inner fail"))
        match r {
            Ok(inner) => {
                match inner {
                    Ok(v) => 1
                    Err(_) => -1
                }
            }
            Err(_) => -2
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn result_match_with_computation_in_arm() {
    ShapeTest::new(
        r#"
        fn safe_div(a, b) -> Result<number> {
            if b == 0 { Err("division by zero") }
            else { Ok(a / b) }
        }
        match safe_div(10, 2) {
            Ok(v) => v * 10
            Err(_) => 0
        }
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn result_match_division_by_zero() {
    ShapeTest::new(
        r#"
        fn safe_div(a, b) -> Result<number> {
            if b == 0 { Err("division by zero") }
            else { Ok(a / b) }
        }
        match safe_div(10, 0) {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn result_used_in_function_return() {
    let code = r#"
fn check(x: int) -> Result<string> {
  if x > 0 {
    Ok("positive")
  } else {
    Err("not positive")
  }
}
print(check(5))
print(check(-1))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("Err");
}

#[test]
fn result_passed_as_function_argument() {
    let code = r#"
fn display_result(r) {
  print(r)
}
display_result(Ok(42))
display_result(Err("nope"))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("Err");
}

#[test]
fn multiple_ok_err_in_sequence() {
    let code = r#"
print(Ok(1))
print(Ok(2))
print(Err("fail"))
print(Ok(3))
"#;
    ShapeTest::new(code).expect_run_ok();
}

// =========================================================================
// Nested Result wrapping / Err idempotence
// =========================================================================

#[test]
fn ok_wrapping_ok_double_wrap() {
    // Ok(Ok(42)) - double wrapping should work
    let code = r#"let x = Ok(Ok(42))
print(x)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok");
}

#[test]
fn err_wrapping_err_nested() {
    // Err(Err("nested")) - nested errors
    let code = r#"let x = Err(Err("nested"))
print(x)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn err_is_idempotent_on_any_error() {
    // Per book: if payload is already an AnyError, reuse it
    let code = r#"
let original = Err("root")
let wrapped = Err(original)
print(wrapped)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

// =========================================================================
// Coalesce operator (??)
// =========================================================================

#[test]
fn coalesce_operator_provides_default_for_none() {
    let code = r#"let maybe_name: Option<string> = None
let name = maybe_name ?? "anonymous"
print(name)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("anonymous");
}

#[test]
fn coalesce_operator_preserves_some_value() {
    let code = r#"let val: Option<int> = Some(42)
let unwrapped = val ?? 0
print(unwrapped)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

#[test]
fn coalesce_on_plain_value_passes_through() {
    ShapeTest::new("let x = 10 ?? 42\nprint(x)")
        .expect_run_ok()
        .expect_output("10");
}

#[test]
fn coalesce_on_none_literal() {
    ShapeTest::new("print(None ?? 42)")
        .expect_run_ok()
        .expect_output("42");
}

#[test]
fn coalesce_on_non_option_value() {
    // ?? on a plain value (non-Option) should pass through
    ShapeTest::new("let x = 10\nlet y = x ?? 99\nprint(y)")
        .expect_run_ok()
        .expect_output("10");
}

#[test]
fn coalesce_with_expression_default() {
    let code = r#"
let x: Option<int> = None
let y = x ?? (1 + 2 + 3)
print(y)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("6");
}

// =========================================================================
// Option propagation / coalesce
// =========================================================================

#[test]
fn option_some_coalesces_to_value() {
    let code = r#"let val: Option<int> = Some(42)
let unwrapped = val ?? 0
print(unwrapped)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

#[test]
fn option_none_coalesces_to_default() {
    let code = r#"let val: Option<int> = None
let unwrapped = val ?? 0
print(unwrapped)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("0");
}

#[test]
fn option_with_conditional_some() {
    let code = r#"
fn maybe_double(x: int) -> Option<int> {
  if x > 0 { Some(x * 2) } else { None }
}
let result = maybe_double(5) ?? -1
print(result)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("10");
}

#[test]
fn option_with_conditional_none() {
    let code = r#"
fn maybe_double(x: int) -> Option<int> {
  if x > 0 { Some(x * 2) } else { None }
}
let result = maybe_double(-3) ?? -1
print(result)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("-1");
}

// =========================================================================
// Result/Option in f-string interpolation
// =========================================================================

#[test]
fn err_in_fstring_interpolation() {
    let code = r#"let e = Err("oops")
print(f"Result is: {e}")
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn ok_in_fstring_interpolation() {
    let code = r#"let r = Ok(42)
print(f"Result is: {r}")
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok");
}
