//! Tests for the try operator (?) in Shape.
//!
//! Covers: basic unwrapping of Ok/Err, propagation through functions,
//! multiple ? in same function, ? in branches/loops/closures, ? on
//! Some, None, and top-level usage.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic ? operator
// =========================================================================

#[test]
fn try_operator_unwraps_ok_value() {
    let code = r#"
fn might_fail(x: int) -> Result<int> {
  if x < 0 { Err("negative") } else { Ok(x * 2) }
}

fn process() -> Result<int> {
  let val = might_fail(5)?
  Ok(val + 1)
}
print(process())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("11");
}

#[test]
fn try_operator_propagates_err() {
    let code = r#"
fn might_fail(x: int) -> Result<int> {
  if x < 0 { Err("negative") } else { Ok(x * 2) }
}

fn process_fail() -> Result<int> {
  let val = might_fail(-1)?
  Ok(val + 1)
}
print(process_fail())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn try_op_unwraps_ok_value() {
    ShapeTest::new(
        r#"
        fn get() -> Result<number> { Ok(42) }
        fn run() -> Result<number> {
            let v = get()?
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

#[test]
fn try_op_propagates_err() {
    ShapeTest::new(
        r#"
        fn fail() -> Result<number> { Err("boom") }
        fn run() -> Result<number> {
            let v = fail()?
            Ok(v + 1)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn try_op_on_ok_yields_inner() {
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let v = Ok(10)?
            Ok(v + 5)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn try_op_on_err_skips_rest() {
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let v = Err("stop")?
            Ok(v + 100)
        }
        match run() {
            Ok(v) => v
            Err(_) => -999
        }
    "#,
    )
    .expect_number(-999.0);
}

#[test]
fn try_op_multiple_in_same_function() {
    ShapeTest::new(
        r#"
        fn a() -> Result<number> { Ok(1) }
        fn b() -> Result<number> { Ok(2) }
        fn c() -> Result<number> { Ok(3) }
        fn run() -> Result<number> {
            let x = a()?
            let y = b()?
            let z = c()?
            Ok(x + y + z)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn try_op_multiple_first_fails() {
    ShapeTest::new(
        r#"
        fn a() -> Result<number> { Err("first") }
        fn b() -> Result<number> { Ok(2) }
        fn run() -> Result<number> {
            let x = a()?
            let y = b()?
            Ok(x + y)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn try_op_multiple_second_fails() {
    ShapeTest::new(
        r#"
        fn a() -> Result<number> { Ok(1) }
        fn b() -> Result<number> { Err("second") }
        fn run() -> Result<number> {
            let x = a()?
            let y = b()?
            Ok(x + y)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn fallible_type_assertion_uses_named_try_into_impl() {
    // The TryInto impl returning Ok(7) IS picked up at runtime;
    // parse_price("n/a") returns Ok(7), so match takes the Ok path.
    ShapeTest::new(
        r#"
        impl TryInto<int> for string as int {
            method tryInto() {
                Ok(7)
            }
        }

        fn parse_price(raw: string) -> Result<int> {
            let n = (raw as int?)?
            Ok(n)
        }

        match parse_price("n/a") {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn fallible_type_assertion_propagates_conversion_failure() {
    ShapeTest::new(
        r#"
        impl TryInto<int> for string as int {
            method tryInto() {
                __try_into_int(self)
            }
        }

        fn parse(raw: string) -> Result<int> {
            let n = (raw as int?)?
            Ok(n)
        }

        match parse("not-int") {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn infallible_type_assertion_uses_into_impl() {
    ShapeTest::new(
        r#"
        impl Into<int> for bool as int {
            method into() {
                __into_int(self)
            }
        }

        let x = true as int
        x
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn infallible_type_assertion_missing_into_impl_errors() {
    ShapeTest::new(
        r#"
        let x = "not-int" as int
        x
    "#,
    )
    .expect_semantic_diagnostic_contains("Cannot assert type");
}

// =========================================================================
// ? in control flow constructs
// =========================================================================

#[test]
fn try_op_in_if_true_branch() {
    ShapeTest::new(
        r#"
        fn get() -> Result<number> { Ok(42) }
        fn run() -> Result<number> {
            if true {
                let v = get()?
                Ok(v)
            } else {
                Ok(0)
            }
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn try_op_in_if_false_branch() {
    ShapeTest::new(
        r#"
        fn get() -> Result<number> { Ok(42) }
        fn run() -> Result<number> {
            if false {
                Ok(0)
            } else {
                let v = get()?
                Ok(v)
            }
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn try_op_in_if_err_propagates() {
    ShapeTest::new(
        r#"
        fn fail() -> Result<number> { Err("nope") }
        fn run() -> Result<number> {
            if true {
                let v = fail()?
                Ok(v)
            } else {
                Ok(0)
            }
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn try_op_in_loop() {
    ShapeTest::new(
        r#"
        fn check(n) -> Result<number> {
            if n == 3 { Err("stop") }
            else { Ok(n) }
        }
        fn run() -> Result<number> {
            let mut sum = 0
            for i in [1, 2, 3, 4] {
                let v = check(i)?
                sum = sum + v
            }
            Ok(sum)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn try_op_in_loop_all_ok() {
    ShapeTest::new(
        r#"
        fn check(n) -> Result<number> { Ok(n * 2) }
        fn run() -> Result<number> {
            let mut sum = 0
            for i in [1, 2, 3] {
                let v = check(i)?
                sum = sum + v
            }
            Ok(sum)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn try_op_in_match_arm() {
    ShapeTest::new(
        r#"
        fn get_val(x) -> Result<number> {
            if x > 0 { Ok(x) } else { Err("negative") }
        }
        fn run(x) -> Result<string> {
            match x {
                1 => {
                    let v = get_val(1)?
                    Ok("one")
                }
                _ => Ok("other")
            }
        }
        match run(1) {
            Ok(v) => v
            Err(_) => "error"
        }
    "#,
    )
    .expect_string("one");
}

// =========================================================================
// ? on inline Ok/Err, nested, plain, Some, None
// =========================================================================

#[test]
fn try_op_on_nested_result() {
    ShapeTest::new(
        r#"
        fn inner() -> Result<number> { Ok(5) }
        fn outer() -> Result<number> {
            let r = inner()
            let v = r?
            Ok(v * 2)
        }
        match outer() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn try_op_on_inline_ok() {
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let v = Ok(7)?
            Ok(v)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn try_op_on_inline_err() {
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let v = Err("inline error")?
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
fn try_op_on_plain_value_reports_semantic_error() {
    // `?` is only valid on Result/Option-typed operands.
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let v = 42?
            Ok(v)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_semantic_diagnostic_contains("try operator '?' expects Result<T, E> or Option<T>");
}

#[test]
fn try_operator_on_plain_value_is_not_allowed() {
    let code = r#"
fn test() -> Result<int> {
  let x = 42?
  Ok(x)
}
print(test())
"#;
    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("try operator '?' expects Result<T, E> or Option<T>");
}

#[test]
fn try_op_on_none_propagates_err() {
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let v = None?
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
fn try_op_on_some_unwraps_value() {
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let v = Some(99)?
            Ok(v)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn none_try_propagation_returns_err() {
    // None? should early-return Err(AnyError) with code OPTION_NONE
    let code = r#"
fn test() -> Result<int> {
  let x = None?
  Ok(x)
}
print(test())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn some_try_unwraps_to_value() {
    let code = r#"
fn test() -> Result<int> {
  let x = Some(42)?
  Ok(x)
}
print(test())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("42");
}

// =========================================================================
// ? at top level
// =========================================================================

#[test]
fn try_op_at_top_level_ok() {
    // ? at top level on Ok should unwrap
    ShapeTest::new(
        r#"
        Ok(42)?
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn try_op_at_top_level_err_fails() {
    // ? at top level on Err should produce runtime error
    ShapeTest::new(
        r#"
        Err("top level error")?
    "#,
    )
    .expect_run_err_contains("top level error");
}

#[test]
fn try_op_at_top_level_none_fails() {
    // ? at top level on None should produce runtime error
    ShapeTest::new(
        r#"
        None?
    "#,
    )
    .expect_run_err_contains("None");
}

#[test]
fn err_propagated_at_top_level_is_uncaught_exception() {
    // If ? is used at top-level on an Err, it should be an uncaught exception
    let code = r#"
fn failing() -> Result<int> { Err("top level error") }
failing()?
"#;
    ShapeTest::new(code).expect_run_err_contains("top level error");
}

// =========================================================================
// ? with chaining and closures
// =========================================================================

#[test]
fn try_op_chained_function_calls() {
    ShapeTest::new(
        r#"
        fn step1() -> Result<number> { Ok(10) }
        fn step2(x) -> Result<number> { Ok(x + 20) }
        fn run() -> Result<number> {
            let a = step1()?
            let b = step2(a)?
            Ok(b)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn try_op_in_closure() {
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let f = |x| Ok(x * 3)?
            Ok(f(5))
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn chain_of_try_operators_all_ok() {
    let code = r#"
fn step1() -> Result<int> { Ok(1) }
fn step2() -> Result<int> { Ok(2) }
fn step3() -> Result<int> { Ok(3) }

fn chain() -> Result<int> {
  let a = step1()?
  let b = step2()?
  let c = step3()?
  Ok(a + b + c)
}
print(chain())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("6");
}

#[test]
fn chain_of_try_operators_middle_fails() {
    let code = r#"
fn step1() -> Result<int> { Ok(1) }
fn step2() -> Result<int> { Err("step2 failed") }
fn step3() -> Result<int> { Ok(3) }

fn chain() -> Result<int> {
  let a = step1()?
  let b = step2()?
  let c = step3()?
  Ok(a + b + c)
}
print(chain())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}

#[test]
fn nested_function_calls_with_try() {
    let code = r#"
fn inner() -> Result<int> { Ok(10) }
fn middle() -> Result<int> {
  let x = inner()?
  Ok(x + 5)
}
fn outer() -> Result<int> {
  let x = middle()?
  Ok(x + 1)
}
print(outer())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("16");
}

#[test]
fn nested_function_calls_inner_fails() {
    let code = r#"
fn inner() -> Result<int> { Err("inner failed") }
fn middle() -> Result<int> {
  let x = inner()?
  Ok(x + 5)
}
fn outer() -> Result<int> {
  let x = middle()?
  Ok(x + 1)
}
print(outer())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}
