//! Edge case tests for error handling in Shape.
//!
//! Covers: explicit return Ok/Err, tail expressions, deeply nested try,
//! error messages with special characters, Option/Result interop, type
//! preservation, loops with results, match arms, chained operations,
//! complex values, nested functions, and more.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Explicit return and tail expressions
// =========================================================================

#[test]
fn edge_return_ok_explicit() {
    ShapeTest::new(
        r#"
        fn f() -> Result<number> {
            return Ok(42)
        }
        match f() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn edge_return_err_explicit() {
    ShapeTest::new(
        r#"
        fn f() -> Result<number> {
            return Err("explicit error")
        }
        match f() {
            Ok(v) => 0
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

#[test]
fn edge_ok_as_tail_expression() {
    ShapeTest::new(
        r#"
        fn f() -> Result<number> {
            Ok(99)
        }
        match f() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn edge_err_as_tail_expression() {
    ShapeTest::new(
        r#"
        fn f() -> Result<number> {
            Err("tail error")
        }
        match f() {
            Ok(v) => 0
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Deeply nested try
// =========================================================================

#[test]
fn edge_deeply_nested_try_five_levels() {
    ShapeTest::new(
        r#"
        fn f1() -> Result<number> { Ok(2) }
        fn f2() -> Result<number> { Ok(f1()? + 3) }
        fn f3() -> Result<number> { Ok(f2()? * 2) }
        fn f4() -> Result<number> { Ok(f3()? - 1) }
        fn f5() -> Result<number> { Ok(f4()? + 10) }
        match f5() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(19.0);
}

// =========================================================================
// Error messages with special characters
// =========================================================================

#[test]
fn edge_error_message_with_special_chars() {
    ShapeTest::new(
        r#"
        Err("error: 'file not found' at /tmp/a.txt")?
    "#,
    )
    .expect_run_err_contains("file not found");
}

#[test]
fn edge_error_message_with_quotes() {
    ShapeTest::new(
        r#"
        Err("expected \"value\" but got nothing")?
    "#,
    )
    .expect_run_err_contains("expected");
}

#[test]
fn edge_error_message_with_newlines() {
    ShapeTest::new(
        r#"
        Err("line1\nline2")?
    "#,
    )
    .expect_run_err_contains("line1");
}

#[test]
fn edge_very_long_error_message() {
    ShapeTest::new(r#"
        Err("This is a very long error message that contains a lot of detail about what went wrong during the processing of the data and should be preserved intact when propagated through multiple levels of the call stack")?
    "#)
    .expect_run_err_contains("very long error message");
}

// =========================================================================
// Ok/None coexistence and Option/Result interop
// =========================================================================

#[test]
fn edge_ok_none_coexistence() {
    // Ok and None are distinct
    ShapeTest::new(
        r#"
        let a = Ok(None)
        match a {
            Ok(v) => "got ok"
            Err(_) => "got err"
        }
    "#,
    )
    .expect_string("got ok");
}

#[test]
fn edge_option_and_result_interop() {
    ShapeTest::new(
        r#"
        fn find_user() { None }
        fn run() -> Result<number> {
            let user = (find_user() !! "User not found")?
            Ok(user)
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
fn edge_option_some_and_result_interop() {
    ShapeTest::new(
        r#"
        fn find_user() { Some(42) }
        fn run() -> Result<number> {
            let user = (find_user() !! "User not found")?
            Ok(user)
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
fn edge_result_with_option_some_via_context() {
    ShapeTest::new(
        r#"
        let x = Some(10)
        let val = x !! "missing"
        match val {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn edge_result_with_option_none_via_context() {
    ShapeTest::new(
        r#"
        let x = None
        let val = x !! "missing"
        match val {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Try on Some/None
// =========================================================================

#[test]
fn edge_try_on_some_value() {
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let v = Some(42)?
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
fn edge_try_on_none_returns_err() {
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

// =========================================================================
// Results in loops
// =========================================================================

#[test]
fn edge_result_in_while_loop() {
    ShapeTest::new(
        r#"
        fn check(n) -> Result<number> {
            if n > 5 { Err("too big") }
            else { Ok(n) }
        }
        fn run() -> Result<number> {
            let mut i = 0
            let mut sum = 0
            while i < 10 {
                let v = check(i)?
                sum = sum + v
                i = i + 1
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
fn edge_result_in_while_loop_all_ok() {
    ShapeTest::new(
        r#"
        fn check(n) -> Result<number> { Ok(n) }
        fn run() -> Result<number> {
            let mut i = 0
            let mut sum = 0
            while i < 5 {
                let v = check(i)?
                sum = sum + v
                i = i + 1
            }
            Ok(sum)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// Match arms with results
// =========================================================================

#[test]
fn edge_multiple_match_arms_with_result() {
    ShapeTest::new(
        r#"
        fn classify(x) -> Result<string> {
            if x < 0 { Err("negative") }
            else if x == 0 { Ok("zero") }
            else { Ok("positive") }
        }
        let result = classify(5)
        match result {
            Ok(v) => v
            Err(_) => "error"
        }
    "#,
    )
    .expect_string("positive");
}

#[test]
fn edge_multiple_match_arms_zero() {
    ShapeTest::new(
        r#"
        fn classify(x) -> Result<string> {
            if x < 0 { Err("negative") }
            else if x == 0 { Ok("zero") }
            else { Ok("positive") }
        }
        match classify(0) {
            Ok(v) => v
            Err(_) => "error"
        }
    "#,
    )
    .expect_string("zero");
}

#[test]
fn edge_multiple_match_arms_negative() {
    ShapeTest::new(
        r#"
        fn classify(x) -> Result<string> {
            if x < 0 { Err("negative") }
            else if x == 0 { Ok("zero") }
            else { Ok("positive") }
        }
        match classify(-5) {
            Ok(v) => v
            Err(_) => "error"
        }
    "#,
    )
    .expect_string("error");
}

// =========================================================================
// Context then match recovery
// =========================================================================

#[test]
fn edge_context_then_match_recovery() {
    ShapeTest::new(
        r#"
        fn risky() -> Result<number> { Err("low") }
        let result = risky() !! "high context"
        match result {
            Ok(v) => v
            Err(_) => 0
        }
    "#,
    )
    .expect_number(0.0);
}

// =========================================================================
// Type preservation through try
// =========================================================================

#[test]
fn edge_try_preserves_value_type_number() {
    ShapeTest::new(
        r#"
        fn get() -> Result<number> { Ok(3.14) }
        fn run() -> Result<number> {
            let v = get()?
            Ok(v)
        }
        match run() {
            Ok(v) => v
            Err(_) => 0.0
        }
    "#,
    )
    .expect_number(3.14);
}

#[test]
fn edge_try_preserves_value_type_string() {
    ShapeTest::new(
        r#"
        fn get() -> Result<string> { Ok("hello") }
        fn run() -> Result<string> {
            let v = get()?
            Ok(v)
        }
        match run() {
            Ok(v) => v
            Err(_) => "error"
        }
    "#,
    )
    .expect_string("hello");
}

#[test]
fn edge_try_preserves_value_type_bool() {
    ShapeTest::new(
        r#"
        fn get() -> Result<bool> { Ok(true) }
        fn run() -> Result<bool> {
            let v = get()?
            Ok(v)
        }
        match run() {
            Ok(v) => v
            Err(_) => false
        }
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// Err in array iteration
// =========================================================================

#[test]
fn edge_err_in_array_iteration() {
    ShapeTest::new(
        r#"
        let items = [Ok(1), Ok(2), Err("bad"), Ok(4)]
        let mut count = 0
        for item in items {
            match item {
                Ok(_) => { count = count + 1 }
                Err(_) => {}
            }
        }
        count
    "#,
    )
    .expect_number(3.0);
}

// =========================================================================
// Chained operations
// =========================================================================

#[test]
fn edge_chained_ok_operations() {
    ShapeTest::new(
        r#"
        fn add_one(x) -> Result<number> { Ok(x + 1) }
        fn double(x) -> Result<number> { Ok(x * 2) }
        fn pipeline() -> Result<number> {
            let a = add_one(0)?
            let b = double(a)?
            let c = add_one(b)?
            let d = double(c)?
            Ok(d)
        }
        match pipeline() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn edge_sequential_fallible_operations() {
    ShapeTest::new(
        r#"
        fn op1() -> Result<number> { Ok(10) }
        fn op2() -> Result<number> { Ok(20) }
        fn op3() -> Result<number> { Ok(30) }

        fn run() -> Result<number> {
            let a = op1()?
            let b = op2()?
            let c = op3()?
            Ok(a + b + c)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn edge_early_return_skips_subsequent_operations() {
    ShapeTest::new(
        r#"
        fn run() -> Result<number> {
            let a = Ok(1)?
            let b = Err("stop here")?
            let c = Ok(3)?
            Ok(a + b + c)
        }
        match run() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Context with number / string interpolation
// =========================================================================

#[test]
fn edge_context_with_number_context_message() {
    // The context message can be a string expression
    ShapeTest::new(
        r#"
        let code = 404
        let r = Err("not found") !! "HTTP error {code}"
        match r {
            Ok(_) => "ok"
            Err(_) => "err"
        }
    "#,
    )
    .expect_string("err");
}

// =========================================================================
// Result used with print
// =========================================================================

#[test]
fn edge_result_used_with_print() {
    ShapeTest::new(
        r#"
        let r = Ok(42)
        match r {
            Ok(v) => print(v)
            Err(e) => print("error")
        }
    "#,
    )
    .expect_output("42");
}

#[test]
fn edge_result_err_used_with_print() {
    ShapeTest::new(
        r#"
        let r = Err("oops")
        match r {
            Ok(v) => print("ok")
            Err(e) => print("caught error")
        }
    "#,
    )
    .expect_output("caught error");
}

// =========================================================================
// Result as if-condition value
// =========================================================================

#[test]
fn edge_result_as_if_condition_value() {
    ShapeTest::new(
        r#"
        fn get() -> Result<number> { Ok(42) }
        let r = get()
        let val = match r {
            Ok(v) => v
            Err(_) => 0
        }
        if val > 10 { "big" } else { "small" }
    "#,
    )
    .expect_string("big");
}

// =========================================================================
// Nested context operators
// =========================================================================

#[test]
fn edge_nested_context_operators() {
    ShapeTest::new(
        r#"
        fn fail() -> Result<number> { Err("root cause") }
        fn layer1() -> Result<number> {
            let v = (fail() !! "layer 1 context")?
            Ok(v)
        }
        fn layer2() -> Result<number> {
            let v = (layer1() !! "layer 2 context")?
            Ok(v)
        }
        match layer2() {
            Ok(v) => 0
            Err(_) => -1
        }
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Complex value types
// =========================================================================

#[test]
fn edge_ok_with_complex_value() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        fn get_point() -> Result<Point> {
            Ok(Point { x: 3, y: 4 })
        }
        match get_point() {
            Ok(p) => p.x + p.y
            Err(_) => -1
        }
    "#,
    )
    .expect_number(7.0);
}

// =========================================================================
// Try in nested function
// =========================================================================

#[test]
fn edge_try_in_nested_function() {
    ShapeTest::new(
        r#"
        fn inner() -> Result<number> { Ok(42) }
        fn outer() -> Result<number> {
            fn nested() -> Result<number> {
                let v = inner()?
                Ok(v + 1)
            }
            let r = nested()?
            Ok(r)
        }
        match outer() {
            Ok(v) => v
            Err(_) => -1
        }
    "#,
    )
    .expect_number(43.0);
}
