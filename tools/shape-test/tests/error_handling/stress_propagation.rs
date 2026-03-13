//! Stress tests for error propagation, Result from functions, runtime errors,
//! compile errors, Result in loops, and early return patterns.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 9: Result from function
// =============================================================================

/// Function returning Ok.
#[test]
fn fn_returning_ok() {
    ShapeTest::new(
        r#"fn test() -> int { let r = Ok(10)
match r { Ok(v) => v, Err(e) => -1 } }
test()"#,
    )
    .expect_number(10.0);
}

/// Function returning Err.
#[test]
fn fn_returning_err() {
    ShapeTest::new(
        r#"fn test() -> int { let r = Err("bad")
match r { Ok(v) => 1, Err(e) => -1 } }
test()"#,
    )
    .expect_number(-1.0);
}

/// Function conditional Ok or Err — Ok path.
#[test]
fn fn_conditional_ok_or_err() {
    ShapeTest::new(
        r#"fn maybe(flag: bool) {
    if flag { return Ok(42) }
    return Err("nope")
}
fn test() -> int {
    let r = maybe(true)
    match r { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(42.0);
}

/// Function conditional returns Err path.
#[test]
fn fn_conditional_returns_err_path() {
    ShapeTest::new(
        r#"fn maybe(flag: bool) {
    if flag { return Ok(42) }
    return Err("nope")
}
fn test() -> int {
    let r = maybe(false)
    match r { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(-1.0);
}

// =============================================================================
// SECTION 14: Multiple error paths in function
// =============================================================================

/// Function multiple err first check.
#[test]
fn fn_multiple_err_first_check() {
    ShapeTest::new(
        r#"fn validate(x: int) {
    if x < 0 { return Err("negative") }
    if x > 100 { return Err("too large") }
    return Ok(x)
}
fn test() -> int {
    let r = validate(-5)
    match r { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(-1.0);
}

/// Function multiple err second check.
#[test]
fn fn_multiple_err_second_check() {
    ShapeTest::new(
        r#"fn validate(x: int) {
    if x < 0 { return Err("negative") }
    if x > 100 { return Err("too large") }
    return Ok(x)
}
fn test() -> int {
    let r = validate(200)
    match r { Ok(v) => v, Err(e) => -2 }
}
test()"#,
    )
    .expect_number(-2.0);
}

/// Function multiple err happy path.
#[test]
fn fn_multiple_err_happy_path() {
    ShapeTest::new(
        r#"fn validate(x: int) {
    if x < 0 { return Err("negative") }
    if x > 100 { return Err("too large") }
    return Ok(x)
}
fn test() -> int {
    let r = validate(50)
    match r { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(50.0);
}

// =============================================================================
// SECTION 15: Error propagation patterns (match and re-wrap)
// =============================================================================

/// Error propagation rewrap.
#[test]
fn error_propagation_rewrap() {
    ShapeTest::new(
        r#"fn inner() { return Err("inner error") }
fn outer() {
    let r = inner()
    match r {
        Ok(v) => Ok(v),
        Err(e) => Err("outer: wrapped")
    }
}
fn test() -> int {
    let r = outer()
    match r { Ok(v) => 1, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(-1.0);
}

/// Error propagation ok passthrough.
#[test]
fn error_propagation_ok_passthrough() {
    ShapeTest::new(
        r#"fn inner() { return Ok(99) }
fn outer() {
    let r = inner()
    match r {
        Ok(v) => Ok(v),
        Err(e) => Err("wrapped")
    }
}
fn test() -> int {
    let r = outer()
    match r { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(99.0);
}

// =============================================================================
// SECTION 17: Runtime errors
// =============================================================================

/// Division by zero int fails.
#[test]
fn division_by_zero_int_fails() {
    ShapeTest::new("fn test() -> int { 1 / 0 }\ntest()").expect_run_err();
}

/// Float division by zero produces a runtime error in Shape.
#[test]
fn division_by_zero_float_is_inf_or_error() {
    ShapeTest::new("1.0 / 0.0").expect_run_err();
}

/// Index out of bounds returns null (not an error).
#[test]
fn index_out_of_bounds_fails() {
    ShapeTest::new("let arr = [1, 2, 3]\narr[10]").expect_none();
}

/// Negative index out of bounds returns null (not an error).
#[test]
fn negative_index_out_of_bounds_fails() {
    ShapeTest::new("let arr = [1, 2, 3]\narr[-10]").expect_none();
}

// =============================================================================
// SECTION 18: Compile errors
// =============================================================================

/// Undefined variable fails.
#[test]
fn undefined_variable_fails() {
    ShapeTest::new("undefined_var").expect_run_err();
}

/// Undefined function call fails.
#[test]
fn undefined_function_call_fails() {
    ShapeTest::new("not_a_function()").expect_run_err();
}

// =============================================================================
// SECTION 21: Result in loops
// =============================================================================

/// Result accumulation in loop.
#[test]
fn result_accumulation_in_loop() {
    ShapeTest::new(
        r#"fn maybe_add(x: int) {
    if x < 0 { return Err("negative") }
    return Ok(x)
}
fn test() -> int {
    let mut sum = 0
    for i in [1, 2, 3, 4, 5] {
        let r = maybe_add(i)
        match r {
            Ok(v) => { sum = sum + v },
            Err(e) => { sum = sum }
        }
    }
    sum
}
test()"#,
    )
    .expect_number(15.0);
}

// =============================================================================
// SECTION 36: Early return with Result
// =============================================================================

/// Early return on err — happy path.
#[test]
fn early_return_on_err() {
    ShapeTest::new(
        r#"fn process(x: int) {
    if x < 0 { return Err("negative input") }
    return Ok(x * 2)
}
fn test() -> int {
    let r = process(5)
    match r { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(10.0);
}

/// Early return triggers err path.
#[test]
fn early_return_triggers_err_path() {
    ShapeTest::new(
        r#"fn process(x: int) {
    if x < 0 { return Err("negative input") }
    return Ok(x * 2)
}
fn test() -> int {
    let r = process(-3)
    match r { Ok(v) => v, Err(e) => -1 }
}
test()"#,
    )
    .expect_number(-1.0);
}
