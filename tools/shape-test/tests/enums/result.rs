use shape_test::shape_test::ShapeTest;

// =========================================================================
// Result<T> — Built-in Generic Enum
// =========================================================================

#[test]
fn builtin_result_ok_and_err() {
    let code = r#"
let a = Ok(10)
let b = Err("fail")
print(a)
print(b)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("10");
}

// =========================================================================
// Try Operator
// =========================================================================

#[test]
fn try_operator_on_ok_result() {
    let code = r#"
fn get_value() -> Result<int> {
  return Ok(42)
}
let x = get_value()?
print(x)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

#[test]
fn try_operator_on_err_result() {
    let code = r#"
fn get_value() -> Result<int> {
  return Err("something went wrong")
}
get_value()?
"#;
    ShapeTest::new(code).expect_run_err_contains("something went wrong");
}

// =========================================================================
// Result<T> — Construction and Basic Usage (from programs_enums_and_matching)
// =========================================================================

#[test]
fn test_result_ok_construction() {
    ShapeTest::new(
        r#"
        let x = Ok(42)
        print(x)
    "#,
    )
    .expect_output_contains("Ok");
}

#[test]
fn test_result_err_construction() {
    ShapeTest::new(
        r#"
        let x = Err("fail")
        print(x)
    "#,
    )
    .expect_output_contains("Err");
}

#[test]
fn test_result_match_ok() {
    ShapeTest::new(
        r#"
        let x = Ok(42)
        match x {
            Ok(val) => val,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_result_match_err() {
    ShapeTest::new(
        r#"
        let x = Err("bad")
        match x {
            Ok(val) => 0,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_result_as_function_return_ok() {
    ShapeTest::new(
        r#"
        fn divide(a, b) {
            if b == 0 { Err("div by zero") } else { Ok(a / b) }
        }
        match divide(10, 2) {
            Ok(v) => v,
            Err(e) => 0
        }
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_result_as_function_return_err() {
    ShapeTest::new(
        r#"
        fn divide(a, b) {
            if b == 0 { Err("div by zero") } else { Ok(a / b) }
        }
        match divide(10, 0) {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_result_ok_with_string() {
    ShapeTest::new(
        r#"
        let r = Ok("success")
        match r {
            Ok(s) => s,
            Err(e) => "fail"
        }
    "#,
    )
    .expect_string("success");
}

#[test]
fn test_result_err_with_string_payload() {
    ShapeTest::new(
        r#"
        let r = Err("something broke")
        match r {
            Ok(v) => "ok",
            Err(e) => "error"
        }
    "#,
    )
    .expect_string("error");
}

#[test]
fn test_result_ok_with_computation() {
    ShapeTest::new(
        r#"
        let r = Ok(7)
        match r {
            Ok(v) => v * v + 1,
            Err(e) => 0
        }
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn test_result_in_conditional() {
    ShapeTest::new(
        r#"
        fn parse_int(s) {
            if s == "42" { Ok(42) } else { Err("invalid") }
        }
        let a = match parse_int("42") {
            Ok(v) => v,
            Err(e) => 0
        }
        let b = match parse_int("abc") {
            Ok(v) => v,
            Err(e) => 0
        }
        a + b
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_result_chained_ok() {
    ShapeTest::new(
        r#"
        fn step1() { Ok(10) }
        fn step2(x) { Ok(x + 5) }
        let r1 = step1()
        let val = match r1 {
            Ok(v) => match step2(v) {
                Ok(v2) => v2,
                Err(e) => -1
            },
            Err(e) => -1
        }
        val
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_result_chained_err_short_circuit() {
    ShapeTest::new(
        r#"
        fn step1() { Err("step1 failed") }
        fn step2(x) { Ok(x + 5) }
        let val = match step1() {
            Ok(v) => match step2(v) {
                Ok(v2) => v2,
                Err(e) => -2
            },
            Err(e) => -1
        }
        val
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_result_ok_bool() {
    ShapeTest::new(
        r#"
        let r = Ok(true)
        match r {
            Ok(b) => b,
            Err(e) => false
        }
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_result_in_array() {
    ShapeTest::new(
        r#"
        let results = [Ok(1), Err("bad"), Ok(3)]
        let sum = 0
        for r in results {
            sum = sum + match r {
                Ok(v) => v,
                Err(e) => 0
            }
        }
        sum
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn test_result_ok_zero_value() {
    ShapeTest::new(
        r#"
        let r = Ok(0)
        match r {
            Ok(v) => v + 100,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(100.0);
}

// =========================================================================
// Try & Context Operators
// =========================================================================

#[test]
fn test_try_operator_on_ok() {
    ShapeTest::new(
        r#"
        fn get_value() -> Result<int> {
            let x = Ok(42)?
            Ok(x)
        }
        match get_value() {
            Ok(v) => v,
            Err(e) => 0
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_try_operator_on_err_propagates() {
    ShapeTest::new(
        r#"
        fn might_fail() -> Result<int> {
            Err("nope")
        }
        fn caller() -> Result<int> {
            let v = might_fail()?
            Ok(v + 1)
        }
        match caller() {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_context_operator_on_err() {
    ShapeTest::new(
        r#"
        let r = Err("low level") !! "high level context"
        print(r)
    "#,
    )
    .expect_output_contains("high level");
}

#[test]
fn test_context_operator_on_ok_passes_through() {
    ShapeTest::new(
        r#"
        let r = Ok(42) !! "should not appear"
        match r {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_combined_context_and_try() {
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
fn test_try_operator_multiple_in_sequence() {
    ShapeTest::new(
        r#"
        fn step1() -> Result<int> { Ok(10) }
        fn step2(x) -> Result<int> { Ok(x + 5) }
        fn pipeline() -> Result<int> {
            let a = step1()?
            let b = step2(a)?
            Ok(b)
        }
        match pipeline() {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_try_operator_early_return_in_sequence() {
    ShapeTest::new(
        r#"
        fn step1() -> Result<int> { Ok(10) }
        fn step2(x) -> Result<int> { Err("step2 failed") }
        fn step3(x) -> Result<int> { Ok(x + 100) }
        fn pipeline() -> Result<int> {
            let a = step1()?
            let b = step2(a)?
            let c = step3(b)?
            Ok(c)
        }
        match pipeline() {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_try_in_function_returning_result_ok() {
    ShapeTest::new(
        r#"
        fn safe_div(a, b) -> Result<number> {
            if b == 0 { return Err("div by zero") }
            Ok(a / b)
        }
        fn compute() -> Result<number> {
            let x = safe_div(100, 4)?
            Ok(x + 1)
        }
        match compute() {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(26.0);
}

#[test]
fn test_try_in_function_returning_result_err() {
    ShapeTest::new(
        r#"
        fn safe_div(a, b) -> Result<number> {
            if b == 0 { return Err("div by zero") }
            Ok(a / b)
        }
        fn compute() -> Result<number> {
            let x = safe_div(100, 0)?
            Ok(x + 1)
        }
        match compute() {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn test_try_in_loop() {
    ShapeTest::new(
        r#"
        fn validate(n) -> Result<int> {
            if n < 0 { Err("negative") } else { Ok(n) }
        }
        fn sum_valid() -> Result<int> {
            let items = [1, 2, 3, 4, 5]
            let total = 0
            for item in items {
                let v = validate(item)?
                total = total + v
            }
            Ok(total)
        }
        match sum_valid() {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(15.0);
}

// =========================================================================
// Result<T> — From Flat File
// =========================================================================

#[test]
fn result_ok_number() {
    ShapeTest::new(
        r#"
        let r = Ok(42)
        match r {
            Ok(v) => v,
            _ => 0
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn result_err_string() {
    ShapeTest::new(
        r#"
        let r = Err("fail")
        match r {
            Ok(v) => "success",
            Err(e) => e
        }
    "#,
    )
    .expect_string("fail");
}

#[test]
fn result_ok_string() {
    ShapeTest::new(
        r#"
        let r = Ok("success")
        match r {
            Ok(v) => v,
            _ => "error"
        }
    "#,
    )
    .expect_string("success");
}

#[test]
fn result_err_takes_error_branch() {
    ShapeTest::new(
        r#"
        let r = Err("broken")
        match r {
            Ok(v) => 1,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn result_ok_takes_success_branch() {
    ShapeTest::new(
        r#"
        let r = Ok(99)
        match r {
            Ok(v) => v + 1,
            _ => 0
        }
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn result_from_function_ok_path() {
    ShapeTest::new(
        r#"
        fn divide(a, b) {
            if b == 0 { Err("division by zero") } else { Ok(a / b) }
        }
        match divide(10, 2) {
            Ok(v) => v,
            Err(e) => -1
        }
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn result_from_function_err_path() {
    ShapeTest::new(
        r#"
        fn divide(a, b) {
            if b == 0 { Err("division by zero") } else { Ok(a / b) }
        }
        match divide(10, 0) {
            Ok(v) => "ok",
            Err(e) => e
        }
    "#,
    )
    .expect_string("division by zero");
}

#[test]
fn result_ok_bool_value() {
    ShapeTest::new(
        r#"
        let r = Ok(true)
        match r {
            Ok(v) => v,
            _ => false
        }
    "#,
    )
    .expect_bool(true);
}

#[test]
fn result_match_as_expression() {
    ShapeTest::new(
        r#"
        let val = match Ok(5) {
            Ok(v) => v * 2,
            _ => 0
        }
        val
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn result_err_match_as_expression() {
    ShapeTest::new(
        r#"
        let val = match Err("oops") {
            Ok(v) => 1,
            Err(e) => -1
        }
        val
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn result_ok_with_computed_value() {
    ShapeTest::new(
        r#"
        let a = 3
        let b = 4
        let r = Ok(a * b)
        match r {
            Ok(v) => v,
            _ => 0
        }
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn result_print_ok() {
    ShapeTest::new(
        r#"
        let r = Ok(42)
        print(r)
    "#,
    )
    .expect_output_contains("Ok");
}

#[test]
fn result_print_err() {
    ShapeTest::new(
        r#"
        let r = Err("fail")
        print(r)
    "#,
    )
    .expect_output_contains("Err");
}

#[test]
fn result_err_with_number_payload() {
    ShapeTest::new(
        r#"
        let r = Err(404)
        match r {
            Ok(v) => 0,
            Err(code) => code
        }
    "#,
    )
    .expect_number(404.0);
}

#[test]
fn result_chained_function_calls() {
    ShapeTest::new(
        r#"
        fn safe_sqrt(n) {
            if n < 0 { Err("negative") } else { Ok(n) }
        }
        fn process(n) {
            match safe_sqrt(n) {
                Ok(v) => v * 2,
                Err(e) => -1
            }
        }
        process(25)
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn result_chained_function_err_propagation() {
    ShapeTest::new(
        r#"
        fn safe_sqrt(n) {
            if n < 0 { Err("negative") } else { Ok(n) }
        }
        fn process(n) {
            match safe_sqrt(n) {
                Ok(v) => v * 2,
                Err(e) => -1
            }
        }
        process(-5)
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn result_try_operator_unwraps_ok() {
    ShapeTest::new(
        r#"
        fn test() -> Result<int> {
            let v = Ok(42)?
            return v
        }
        test()?
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn result_try_operator_propagates_err() {
    ShapeTest::new(
        r#"
        fn test() -> Result<int> {
            let v = Err("broken")?
            return 0
        }
        test()?
    "#,
    )
    .expect_run_err_contains("broken");
}

#[test]
fn result_err_context_with_bang_bang() {
    ShapeTest::new(
        r#"
        fn test() -> Result<int> {
            return Err("base") !! "context"
        }
        test()?
    "#,
    )
    .expect_run_err_contains("context");
}

#[test]
fn result_ok_in_conditional() {
    ShapeTest::new(
        r#"
        let flag = true
        let r = if flag { Ok(1) } else { Err("no") }
        match r {
            Ok(v) => v,
            _ => -1
        }
    "#,
    )
    .expect_number(1.0);
}
