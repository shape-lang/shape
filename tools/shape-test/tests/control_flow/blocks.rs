//! Block expression tests.
//!
//! Covers:
//! - Block as expression (simple, intermediate computation)
//! - Nested blocks
//! - Multiple statements in block (last is value)
//! - Block scope isolation / shadowing
//! - Block with if inside
//! - Block in function
//! - Block as if condition body / match arm body
//! - Multiple sequential blocks
//! - Block with string result
//! - Deeply nested blocks
//! - Block as function argument
//! - Block with loop inside
//! - Trailing semicolons and unit values
//! - Empty block

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Block as expression
// =========================================================================

#[test]
fn block_as_expression_simple() {
    ShapeTest::new(
        r#"
        let x = {
            let a = 1
            let b = 2
            a + b
        }
        x
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn block_as_expression_with_intermediate_computation() {
    ShapeTest::new(
        r#"
        let result = {
            let base = 10
            let multiplier = 5
            let offset = 3
            base * multiplier + offset
        }
        result
    "#,
    )
    .expect_number(53.0);
}

/// A block returns the value of its last expression.
#[test]
fn cf_02_block_return_value() {
    let code = r#"
// Test 02: Blocks return their last expression's value
let value = {
  let base = 10
  base * 2
}
print(value)
// Expected: 20
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("20");
}

// =========================================================================
// Nested blocks
// =========================================================================

#[test]
fn nested_blocks() {
    ShapeTest::new(
        r#"
        let x = {
            let a = {
                let inner = 10
                inner * 2
            }
            a + 1
        }
        x
    "#,
    )
    .expect_number(21.0);
}

/// Deeply nested blocks each return their last expression value.
#[test]
fn cf_16_deeply_nested_blocks() {
    let code = r#"
// Test 16: Deeply nested blocks returning values
let result = {
  let a = {
    let b = {
      let c = 100
      c + 1
    }
    b * 2
  }
  a + 3
}
print(result)
// Expected: 205  (100+1=101, 101*2=202, 202+3=205)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("205");
}

#[test]
fn deeply_nested_blocks() {
    ShapeTest::new(
        r#"
        let x = {
            let a = {
                let b = {
                    let c = 5
                    c * 2
                }
                b + 3
            }
            a + 1
        }
        x
    "#,
    )
    .expect_number(14.0);
}

// =========================================================================
// Multiple statements in block
// =========================================================================

#[test]
fn block_with_multiple_statements_last_is_value() {
    ShapeTest::new(
        r#"
        let x = {
            var temp = 0
            temp = temp + 1
            temp = temp + 2
            temp = temp + 3
            temp
        }
        x
    "#,
    )
    .expect_number(6.0);
}

/// Block with multiple let bindings returns the last expression.
#[test]
fn cf_18_multiple_stmts_block() {
    let code = r#"
// Test 18: Multiple statements in a block, last one is the value
let val = {
  let x = 1
  let y = 2
  let z = 3
  x + y + z
}
print(val)
// Expected: 6
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("6");
}

// =========================================================================
// Block scope isolation and shadowing
// =========================================================================

#[test]
fn block_scope_isolation_no_leak() {
    // Variable defined inside block should not be visible outside
    ShapeTest::new(
        r#"
        let x = 1
        let y = {
            let x = 100
            x + 1
        }
        x + y
    "#,
    )
    .expect_number(102.0);
}

#[test]
fn shadowing_in_block() {
    ShapeTest::new(
        r#"
        let x = 10
        let y = {
            let x = 20
            x
        }
        x + y
    "#,
    )
    .expect_number(30.0);
}

/// Block scoping correctly prevents inner variables from leaking.
/// Accessing `inner` outside the block is a semantic error -- this is
/// correct behavior and the test verifies the error is produced.
#[test]
fn cf_25_block_scope() {
    let code = r#"
// Test 25: Block scoping - variable from inner block should not leak
let x = {
  let inner = 42
  inner
}
print(x)
print(inner)
// Expected: 42, then error on accessing `inner`
"#;
    ShapeTest::new(code).expect_run_err_contains("Undefined variable");
}

/// Block scope -- inner variable does not leak, but the block value is captured.
#[test]
fn cf_25b_block_scope_no_error() {
    let code = r#"
// Test 25b: Block scope - verify inner variable does not leak
let x = {
  let inner = 42
  inner
}
print(x)
// Expected: 42
// Note: Test 25 confirmed accessing `inner` outside the block is a semantic error
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

// =========================================================================
// Block with if inside
// =========================================================================

#[test]
fn block_with_if_inside() {
    ShapeTest::new(
        r#"
        let result = {
            let a = 5
            if a > 3 { "big" } else { "small" }
        }
        result
    "#,
    )
    .expect_string("big");
}

// =========================================================================
// Block in function
// =========================================================================

#[test]
fn block_expression_in_function() {
    ShapeTest::new(
        r#"
        fn compute(x) {
            let intermediate = {
                let doubled = x * 2
                let tripled = x * 3
                doubled + tripled
            }
            intermediate + x
        }
        compute(4)
    "#,
    )
    .expect_number(24.0);
}

// =========================================================================
// Block as if condition body / match arm body
// =========================================================================

#[test]
fn block_as_if_condition_body() {
    // Block inside the then-branch of an if
    ShapeTest::new(
        r#"
        let x = if true {
            let a = 10
            let b = 20
            a * b
        } else {
            0
        }
        x
    "#,
    )
    .expect_number(200.0);
}

#[test]
fn block_as_match_arm_body() {
    ShapeTest::new(
        r#"
        let x = 2
        let result = match x {
            1 => {
                let a = 100
                a
            },
            2 => {
                let b = 200
                b + 1
            },
            _ => 0
        }
        result
    "#,
    )
    .expect_number(201.0);
}

// =========================================================================
// Multiple sequential blocks
// =========================================================================

#[test]
fn multiple_sequential_blocks() {
    ShapeTest::new(
        r#"
        let a = {
            let x = 1
            x + 1
        }
        let b = {
            let x = 10
            x + 1
        }
        a + b
    "#,
    )
    .expect_number(13.0);
}

// =========================================================================
// Block with string result
// =========================================================================

#[test]
fn block_with_string_result() {
    ShapeTest::new(
        r#"
        let greeting = {
            let name = "World"
            "Hello " + name
        }
        greeting
    "#,
    )
    .expect_string("Hello World");
}

// =========================================================================
// Block as function argument
// =========================================================================

#[test]
fn block_expression_as_function_argument() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        let result = double({
            let a = 3
            let b = 4
            a + b
        })
        result
    "#,
    )
    .expect_number(14.0);
}

/// A block expression used directly as a function call argument.
#[test]
fn cf_32_block_expression_in_call() {
    let code = r#"
// Test 32: Block expression used directly in function call
print({ let x = 10; x * 3 })
// Expected: 30 or syntax error
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("30");
}

// =========================================================================
// Block with loop inside
// =========================================================================

#[test]
fn block_with_loop_inside() {
    ShapeTest::new(
        r#"
        let sum = {
            var total = 0
            var i = 1
            while i <= 5 {
                total = total + i
                i = i + 1
            }
            total
        }
        sum
    "#,
    )
    .expect_number(15.0);
}

// =========================================================================
// Trailing semicolons and unit values
// =========================================================================

/// A trailing semicolon in a block discards the value (returns 1 in practice).
#[test]
fn cf_03_trailing_semicolon() {
    let code = r#"
// Test 03: Trailing semicolon discards value (returns unit)
let unit = { 1; }
print(unit)
// Expected: () or some unit representation
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("1");
}

/// Detailed trailing semicolon behavior across various block forms.
#[test]
fn cf_03b_trailing_semicolon_detail() {
    let code = r#"
// Test 03b: Trailing semicolon - more detailed
let a = { 42 }
print(f"a={a}")
// Expected: a=42

let b = { 42; }
print(f"b={b}")
// Expected: b=() -- but based on test 03, might be b=42

let c = { 1; 2; 3 }
print(f"c={c}")
// Expected: c=3

let d = { 1; 2; 3; }
print(f"d={d}")
// Expected: d=() -- if trailing semicolon discards, or d=3
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("a=42\nb=42\nc=3\nd=3");
}

// =========================================================================
// Empty block
// =========================================================================

/// An empty block `{}` evaluates to an empty object.
#[test]
fn cf_08_empty_block() {
    let code = r#"
// Test 08: Empty block
let x = {}
print(x)
// Expected: () or error
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("{}");
}
