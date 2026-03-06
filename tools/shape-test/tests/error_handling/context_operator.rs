//! Tests for the context operator (!!) in Shape.
//!
//! Covers: !! on Err/Ok/None/Some/plain values, combined !! + ? patterns,
//! ergonomic form (lhs !! rhs?), runtime error surfacing, chained context,
//! string interpolation in context messages, and function result context.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic !! operator behavior (from main.rs)
// =========================================================================

#[test]
fn context_operator_wraps_err_with_context() {
    let code = r#"let user_id = (Err("not found") !! "User lookup failed")
print(user_id)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn context_operator_passes_through_ok() {
    let code = r#"let result = (Ok(42) !! "should not appear")
print(result)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("42");
}

#[test]
fn context_operator_on_some_becomes_ok() {
    let code = r#"let result = (Some(99) !! "should not appear")
print(result)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("99");
}

#[test]
fn context_operator_on_none_becomes_err() {
    let code = r#"let result = (None !! "missing value")
print(result)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn context_operator_on_plain_value_becomes_ok() {
    let code = r#"let result = (42 !! "should wrap in Ok")
print(result)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("42");
}

// =========================================================================
// !! with match-based assertions (from programs_error_handling.rs)
// =========================================================================

#[test]
fn context_op_on_err_adds_context() {
    ShapeTest::new(
        r#"
        let r = Err("low level") !! "high level"
        match r {
            Ok(v) => "ok"
            Err(_) => "err"
        }
    "#,
    )
    .expect_string("err");
}

#[test]
fn context_op_on_ok_passes_through() {
    ShapeTest::new(
        r#"
        let r = Ok(42) !! "context message"
        match r {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn context_op_on_none_wraps_to_err() {
    ShapeTest::new(
        r#"
        let r = None !! "value was missing"
        match r {
            Ok(_) => "ok"
            Err(_) => "err"
        }
    "#,
    )
    .expect_string("err");
}

#[test]
fn context_op_on_some_passes_through_as_ok() {
    ShapeTest::new(
        r#"
        let r = Some(99) !! "context"
        match r {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn context_op_on_plain_value_wraps_as_ok() {
    ShapeTest::new(
        r#"
        let r = 42 !! "unused context"
        match r {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// !! + ? combined
// =========================================================================

#[test]
fn context_plus_try_propagates_contextual_error() {
    let code = r#"
fn find_user() { None }

fn main_fn() -> Result<string> {
  let user = (find_user() !! "User not found")?
  Ok(user)
}
print(main_fn())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn context_plus_try_sugar_parsing() {
    // lhs !! rhs? should parse as (lhs !! rhs)?
    let code = r#"
fn main_fn() -> Result<int> {
  let a = Err("low") !! "high"?
  Ok(a)
}
print(main_fn())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn context_op_combined_with_try() {
    ShapeTest::new(
        r#"
        fn fail() -> Result<number> { Err("low") }
        fn run() -> Result<number> {
            let v = (fail() !! "high")?
            Ok(v)
        }
        match run() {
            Ok(v) => 0
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn context_op_combined_with_try_ok_path() {
    ShapeTest::new(
        r#"
        fn succeed() -> Result<number> { Ok(42) }
        fn run() -> Result<number> {
            let v = (succeed() !! "context")?
            Ok(v)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// Ergonomic form: lhs !! rhs?
// =========================================================================

#[test]
fn context_op_ergonomic_form_err_then_try() {
    // lhs !! rhs? parses as (lhs !! rhs)?
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let v = Err("low") !! "high"?
            Ok(v)
        }
        match run() {
            Ok(v) => 0
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn context_op_ergonomic_form_ok_then_try() {
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let v = Ok(50) !! "context"?
            Ok(v)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(50.0);
}

// =========================================================================
// !! error surfacing in runtime errors
// =========================================================================

#[test]
fn context_op_err_surfaces_in_runtime_error() {
    ShapeTest::new(
        r#"
        Err("low level") !! "high level context"?
    "#,
    )
    .expect_run_err_contains("high level context");
}

#[test]
fn context_op_err_preserves_cause() {
    ShapeTest::new(
        r#"
        Err("original cause") !! "wrapper context"?
    "#,
    )
    .expect_run_err_contains("original cause");
}

#[test]
fn context_op_none_surfaces_in_runtime_error() {
    ShapeTest::new(
        r#"
        None !! "missing value"?
    "#,
    )
    .expect_run_err_contains("missing value");
}

#[test]
fn context_op_none_includes_none_cause() {
    ShapeTest::new(
        r#"
        None !! "missing value"?
    "#,
    )
    .expect_run_err_contains("None");
}

// =========================================================================
// Chained !! operators
// =========================================================================

#[test]
fn multiple_context_operators_chain() {
    let code = r#"
fn step1() -> Result<int> { Err("root cause") }

fn step2() -> Result<int> {
  let x = (step1() !! "step2 context")?
  Ok(x)
}

fn step3() -> Result<int> {
  let x = (step2() !! "step3 context")?
  Ok(x)
}
print(step3())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn context_op_multiple_chained() {
    ShapeTest::new(
        r#"
        fn fail() -> Result<number> { Err("root") }
        fn mid() -> Result<number> {
            let v = (fail() !! "mid context")?
            Ok(v)
        }
        fn top() -> Result<number> {
            let v = (mid() !! "top context")?
            Ok(v)
        }
        match top() {
            Ok(v) => 0
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// !! with string interpolation and function results
// =========================================================================

#[test]
fn context_op_with_string_interpolation() {
    ShapeTest::new(
        r#"
        let path = "/etc/config"
        let r = Err("not found") !! "Failed to load {path}"
        match r {
            Ok(_) => "ok"
            Err(_) => "err"
        }
    "#,
    )
    .expect_string("err");
}

#[test]
fn context_op_on_function_result() {
    ShapeTest::new(
        r#"
        fn risky() -> Result<number> { Err("disk full") }
        let r = risky() !! "save failed"
        match r {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn context_op_on_function_result_ok() {
    ShapeTest::new(
        r#"
        fn safe() -> Result<number> { Ok(10) }
        let r = safe() !! "context"
        match r {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// !! with declared Result return type
// =========================================================================

#[test]
fn context_op_declared_result_return_type_with_err() {
    ShapeTest::new(
        r#"
        fn test() -> Result<int> {
            return Err("some error") !! "yes, something went wrong"
        }
        match test() {
            Ok(v) => 0
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn context_op_declared_result_return_type_propagates() {
    ShapeTest::new(
        r#"
        fn test() -> Result<int> {
            return Err("some error") !! "yes, something went wrong"
        }
        test()?
    "#,
    )
    .expect_run_err_contains("yes, something went wrong");
}

#[test]
fn declared_result_return_with_err_context() {
    // Regression from language_surface_regressions.rs
    let code = r#"
fn test() -> Result<int> {
  return Err("some error") !! "yes, something went wrong"
}

test()?
"#;
    ShapeTest::new(code)
        .expect_no_semantic_diagnostics()
        .expect_run_err_contains("yes, something went wrong");
}
