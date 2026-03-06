//! Tests for error propagation patterns in Shape.
//!
//! Covers: multi-level propagation (3 and 5 levels), mixed ok/err paths,
//! error recovery with defaults, context at each level, conditional error
//! paths, accumulated results, recursive functions, and option-to-result
//! coercion with propagation.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Multi-level propagation
// =========================================================================

#[test]
fn propagation_three_levels_ok() {
    ShapeTest::new(
        r#"
        fn level3() -> Result<number> { Ok(1) }
        fn level2() -> Result<number> {
            let v = level3()?
            Ok(v + 10)
        }
        fn level1() -> Result<number> {
            let v = level2()?
            Ok(v + 100)
        }
        match level1() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(111.0);
}

#[test]
fn propagation_three_levels_deepest_fails() {
    ShapeTest::new(
        r#"
        fn level3() -> Result<number> { Err("deep failure") }
        fn level2() -> Result<number> {
            let v = level3()?
            Ok(v + 10)
        }
        fn level1() -> Result<number> {
            let v = level2()?
            Ok(v + 100)
        }
        match level1() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn propagation_three_levels_middle_fails() {
    ShapeTest::new(
        r#"
        fn level3() -> Result<number> { Ok(1) }
        fn level2() -> Result<number> {
            let v = level3()?
            Err("middle failure")
        }
        fn level1() -> Result<number> {
            let v = level2()?
            Ok(v + 100)
        }
        match level1() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn propagation_five_levels_deep() {
    ShapeTest::new(
        r#"
        fn f5() -> Result<number> { Ok(1) }
        fn f4() -> Result<number> { let v = f5()?; Ok(v + 1) }
        fn f3() -> Result<number> { let v = f4()?; Ok(v + 1) }
        fn f2() -> Result<number> { let v = f3()?; Ok(v + 1) }
        fn f1() -> Result<number> { let v = f2()?; Ok(v + 1) }
        match f1() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn propagation_five_levels_deepest_fails() {
    ShapeTest::new(
        r#"
        fn f5() -> Result<number> { Err("deep error") }
        fn f4() -> Result<number> { let v = f5()?; Ok(v + 1) }
        fn f3() -> Result<number> { let v = f4()?; Ok(v + 1) }
        fn f2() -> Result<number> { let v = f3()?; Ok(v + 1) }
        fn f1() -> Result<number> { let v = f2()?; Ok(v + 1) }
        match f1() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Mixed ok/err paths
// =========================================================================

#[test]
fn propagation_mixed_ok_err_paths() {
    ShapeTest::new(
        r#"
        fn check(n) -> Result<number> {
            if n % 2 == 0 { Ok(n) }
            else { Err("odd number") }
        }
        fn process() -> Result<number> {
            let a = check(2)?
            let b = check(4)?
            Ok(a + b)
        }
        match process() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn propagation_mixed_ok_err_second_fails() {
    ShapeTest::new(
        r#"
        fn check(n) -> Result<number> {
            if n % 2 == 0 { Ok(n) }
            else { Err("odd number") }
        }
        fn process() -> Result<number> {
            let a = check(2)?
            let b = check(3)?
            Ok(a + b)
        }
        match process() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Error recovery
// =========================================================================

#[test]
fn propagation_error_recovery_with_default() {
    ShapeTest::new(
        r#"
        fn risky() -> Result<number> { Err("fail") }
        fn safe_wrapper() {
            match risky() {
                Ok(v) => v
                Err(_) => 0
            }
        }
        safe_wrapper() + 1
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn propagation_error_recovery_ok_path() {
    ShapeTest::new(
        r#"
        fn risky() -> Result<number> { Ok(42) }
        fn safe_wrapper() {
            match risky() {
                Ok(v) => v
                Err(_) => 0
            }
        }
        safe_wrapper() + 1
    "#,
    )
    .expect_number(43.0);
}

// =========================================================================
// Context at each propagation level
// =========================================================================

#[test]
fn propagation_with_context_at_each_level() {
    ShapeTest::new(
        r#"
        fn read_file() -> Result<string> { Err("io error") }
        fn parse_config() -> Result<string> {
            let text = (read_file() !! "failed to read config")?
            Ok(text)
        }
        fn init_app() -> Result<string> {
            let cfg = (parse_config() !! "initialization failed")?
            Ok(cfg)
        }
        match init_app() {
            Ok(v) => "ok"
            Err(_) => "err"
        }
    "#,
    )
    .expect_string("err");
}

// =========================================================================
// Conditional error paths
// =========================================================================

#[test]
fn propagation_conditional_error_path() {
    ShapeTest::new(
        r#"
        fn validate(x) -> Result<number> {
            if x < 0 { return Err("negative") }
            if x > 100 { return Err("too large") }
            Ok(x)
        }
        fn process(x) -> Result<number> {
            let v = validate(x)?
            Ok(v * 2)
        }
        match process(50) {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn propagation_conditional_error_path_negative() {
    ShapeTest::new(
        r#"
        fn validate(x) -> Result<number> {
            if x < 0 { return Err("negative") }
            if x > 100 { return Err("too large") }
            Ok(x)
        }
        fn process(x) -> Result<number> {
            let v = validate(x)?
            Ok(v * 2)
        }
        match process(-5) {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn propagation_conditional_error_path_too_large() {
    ShapeTest::new(
        r#"
        fn validate(x) -> Result<number> {
            if x < 0 { return Err("negative") }
            if x > 100 { return Err("too large") }
            Ok(x)
        }
        fn process(x) -> Result<number> {
            let v = validate(x)?
            Ok(v * 2)
        }
        match process(200) {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Accumulated results
// =========================================================================

#[test]
fn propagation_accumulate_results() {
    ShapeTest::new(
        r#"
        fn safe_div(a, b) -> Result<number> {
            if b == 0 { Err("div by zero") }
            else { Ok(a / b) }
        }
        fn compute() -> Result<number> {
            let a = safe_div(10, 2)?
            let b = safe_div(20, 4)?
            let c = safe_div(30, 5)?
            Ok(a + b + c)
        }
        match compute() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(16.0);
}

#[test]
fn propagation_accumulate_results_one_fails() {
    ShapeTest::new(
        r#"
        fn safe_div(a, b) -> Result<number> {
            if b == 0 { Err("div by zero") }
            else { Ok(a / b) }
        }
        fn compute() -> Result<number> {
            let a = safe_div(10, 2)?
            let b = safe_div(20, 0)?
            let c = safe_div(30, 5)?
            Ok(a + b + c)
        }
        match compute() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Recursive functions with error propagation
// =========================================================================

#[test]
fn propagation_error_in_recursive_function() {
    ShapeTest::new(
        r#"
        fn countdown(n) -> Result<number> {
            if n < 0 { return Err("negative") }
            if n == 0 { return Ok(0) }
            let v = countdown(n - 1)?
            Ok(v + n)
        }
        match countdown(5) {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn propagation_error_in_recursive_function_fails() {
    ShapeTest::new(
        r#"
        fn countdown(n) -> Result<number> {
            if n < 0 { return Err("negative") }
            if n == 0 { return Ok(0) }
            let v = countdown(n - 1)?
            Ok(v + n)
        }
        match countdown(-1) {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Option-to-Result coercion with propagation
// =========================================================================

#[test]
fn propagation_try_after_successful_option_coerce() {
    // Use !! to coerce Option to Result instead of bare Some/None match patterns
    ShapeTest::new(
        r#"
        fn get_opt() { Some(10) }
        fn process() -> Result<number> {
            let v = (get_opt() !! "missing")?
            Ok(v + 5)
        }
        match process() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn propagation_try_after_failed_option_coerce() {
    // Use !! to coerce Option to Result instead of bare Some/None match patterns
    ShapeTest::new(
        r#"
        fn get_opt() { None }
        fn process() -> Result<number> {
            let v = (get_opt() !! "missing")?
            Ok(v + 5)
        }
        match process() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Complex combined propagation scenario (from main.rs)
// =========================================================================

#[test]
fn complex_error_handling_scenario() {
    let code = r#"
fn parse_value(s: string) -> Result<int> {
  if s == "42" { Ok(42) } else { Err("invalid input") }
}

fn get_config_value() -> Option<string> {
  Some("42")
}

fn process() -> Result<int> {
  let raw = (get_config_value() !! "config missing")?
  let val = (parse_value(raw) !! "parse failed")?
  Ok(val + 1)
}

print(process())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Ok")
        .expect_output_contains("43");
}

#[test]
fn complex_error_handling_scenario_config_missing() {
    let code = r#"
fn parse_value(s: string) -> Result<int> {
  if s == "42" { Ok(42) } else { Err("invalid input") }
}

fn get_config_value() -> Option<string> {
  None
}

fn process() -> Result<int> {
  let raw = (get_config_value() !! "config missing")?
  let val = (parse_value(raw) !! "parse failed")?
  Ok(val + 1)
}

print(process())
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("Err");
}
