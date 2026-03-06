//! If/else expression and statement tests.
//!
//! Covers:
//! - If expression returns value from matching branch
//! - If without else (true/false conditions)
//! - Nested if expressions as values
//! - Chained else-if
//! - Boolean conditions (&&, ||, !)
//! - If as statement (not expression)
//! - If expression in print / function call
//! - Type mismatch in branches (dynamic typing)

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic if/else
// =========================================================================

#[test]
fn if_true_returns_then_branch() {
    ShapeTest::new(
        r#"
        if true { 1 } else { 0 }
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn if_false_returns_else_branch() {
    ShapeTest::new(
        r#"
        if false { 1 } else { 0 }
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn if_without_else_true_condition() {
    // if without else when condition is true should execute body
    ShapeTest::new(
        r#"
        var x = 0
        if true { x = 42 }
        x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn if_without_else_false_condition() {
    // if without else when condition is false should skip body
    ShapeTest::new(
        r#"
        var x = 0
        if false { x = 42 }
        x
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn if_else_as_expression_assigns_value() {
    ShapeTest::new(
        r#"
        let x = if true { 42 } else { 0 }
        x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn if_else_as_expression_false_branch_value() {
    ShapeTest::new(
        r#"
        let x = if false { 42 } else { 99 }
        x
    "#,
    )
    .expect_number(99.0);
}

// =========================================================================
// If expression returns value (cf_01)
// =========================================================================

/// If expression returns value from the matching branch.
#[test]
fn cf_01_if_expression() {
    let code = r#"
// Test 01: If expressions return values
let score = 84
let grade = if score >= 90 { "A" } else if score >= 80 { "B" } else { "C" }
print(grade)
// Expected: B
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("B");
}

// =========================================================================
// Nested if expressions
// =========================================================================

#[test]
fn nested_if_else_inner_true() {
    ShapeTest::new(
        r#"
        let x = if true { if true { 42 } else { 0 } } else { 99 }
        x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn nested_if_else_inner_false() {
    ShapeTest::new(
        r#"
        let x = if true { if false { 42 } else { 7 } } else { 99 }
        x
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn nested_if_else_outer_false() {
    ShapeTest::new(
        r#"
        let x = if false { 1 } else { if false { 2 } else { 3 } }
        x
    "#,
    )
    .expect_number(3.0);
}

/// Nested if expressions return the correct inner value.
#[test]
fn cf_09_nested_if_value() {
    let code = r#"
// Test 09: Nested if expressions used as values
let a = 5
let b = 10
let result = if a > 3 {
  if b > 8 { "both big" } else { "only a big" }
} else {
  "a small"
}
print(result)
// Expected: both big
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("both big");
}

/// Debug test: nested if value propagation with intermediate variable.
#[test]
fn cf_09b_nested_if_debug() {
    let code = r#"
// Test 09b: Debug nested if value propagation
// First, check the inner if works on its own
let b = 10
let inner = if b > 8 { "both big" } else { "only a big" }
print(inner)
// Expected: both big

// Now wrap it in an outer if
let a = 5
let outer = if a > 3 {
  let r = if b > 8 { "both big" } else { "only a big" }
  r
} else {
  "a small"
}
print(outer)
// Expected: both big
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("both big\nboth big");
}

/// Minimal nested if as last expression in a block.
#[test]
fn cf_09c_nested_if_minimal() {
    let code = r#"
// Test 09c: Minimal nested if as last expression
let x = if true {
  if true { 42 } else { 0 }
} else {
  99
}
print(x)
// Expected: 42
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

/// Various nested if-expression-as-value patterns.
#[test]
fn cf_09d_nested_if_variants() {
    let code = r#"
// Test 09d: Various nested if-expression-as-value patterns

// Variant 1: Nested if as direct tail expression in block (same as 09c, confirming)
let v1 = if true {
  if true { 1 } else { 2 }
} else {
  3
}
print(f"v1={v1}")
// Expected: v1=1, likely v1=None (bug)

// Variant 2: With intermediate binding (workaround)
let v2 = if true {
  let tmp = if true { 1 } else { 2 }
  tmp
} else {
  3
}
print(f"v2={v2}")
// Expected: v2=1

// Variant 3: Non-if expression as tail works fine
let v3 = if true {
  let x = 10
  x + 5
} else {
  3
}
print(f"v3={v3}")
// Expected: v3=15
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("v1=1\nv2=1\nv3=15");
}

#[test]
fn deeply_nested_if_else_chain() {
    ShapeTest::new(
        r#"
        let x = if true {
            if true {
                if true {
                    if true { 42 } else { 0 }
                } else { 0 }
            } else { 0 }
        } else { 0 }
        x
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// Chained else-if
// =========================================================================

#[test]
fn chained_if_else_if_else_first_match() {
    ShapeTest::new(
        r#"
        let x = 10
        let result = if x > 20 { "big" } else if x > 5 { "medium" } else { "small" }
        result
    "#,
    )
    .expect_string("medium");
}

#[test]
fn chained_if_else_if_else_last_match() {
    ShapeTest::new(
        r#"
        let x = 1
        let result = if x > 20 { "big" } else if x > 5 { "medium" } else { "small" }
        result
    "#,
    )
    .expect_string("small");
}

#[test]
fn chained_if_else_if_else_first_branch() {
    ShapeTest::new(
        r#"
        let x = 100
        let result = if x > 20 { "big" } else if x > 5 { "medium" } else { "small" }
        result
    "#,
    )
    .expect_string("big");
}

/// Many chained else-if branches select the correct one.
#[test]
fn cf_24_chained_else_if() {
    let code = r#"
// Test 24: Many chained else-if
let n = 3
let name = if n == 1 { "one" } else if n == 2 { "two" } else if n == 3 { "three" } else if n == 4 { "four" } else { "other" }
print(name)
// Expected: three
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("three");
}

// =========================================================================
// Boolean conditions
// =========================================================================

#[test]
fn if_with_comparison_condition() {
    ShapeTest::new(
        r#"
        let a = 10
        let b = 20
        if a < b { "less" } else { "not less" }
    "#,
    )
    .expect_string("less");
}

#[test]
fn if_with_logical_and_condition() {
    ShapeTest::new(
        r#"
        let x = 15
        if x > 10 and x < 20 { "in range" } else { "out of range" }
    "#,
    )
    .expect_string("in range");
}

#[test]
fn if_with_logical_or_condition() {
    ShapeTest::new(
        r#"
        let x = 5
        if x < 0 or x > 100 { "extreme" } else { "normal" }
    "#,
    )
    .expect_string("normal");
}

#[test]
fn if_with_negation_condition() {
    ShapeTest::new(
        r#"
        let flag = false
        if !flag { "not set" } else { "set" }
    "#,
    )
    .expect_string("not set");
}

#[test]
fn if_with_equality_condition() {
    ShapeTest::new(
        r#"
        let x = 42
        if x == 42 { "the answer" } else { "nope" }
    "#,
    )
    .expect_string("the answer");
}

/// Various boolean operators (&&, ||, !) in if conditions.
#[test]
fn cf_33_if_boolean_conditions() {
    let code = r#"
// Test 33: Various boolean conditions in if
let x = 5
if x > 3 && x < 10 {
  print("in range")
}
if x < 3 || x > 4 {
  print("out or above")
}
if !(x == 5) {
  print("not five")
} else {
  print("is five")
}
// Expected: in range, out or above, is five
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("in range\nout or above\nis five");
}

// =========================================================================
// If with block body
// =========================================================================

#[test]
fn if_with_block_body_multiple_statements() {
    ShapeTest::new(
        r#"
        var total = 0
        if true {
            var a = 10
            var b = 20
            total = a + b
        }
        total
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn if_with_function_call_condition() {
    ShapeTest::new(
        r#"
        fn is_positive(n) { n > 0 }
        if is_positive(5) { "yes" } else { "no" }
    "#,
    )
    .expect_string("yes");
}

// =========================================================================
// If without else
// =========================================================================

/// If without else, condition true -- returns the body value.
#[test]
fn cf_15_if_without_else() {
    let code = r#"
// Test 15: If without else - what does it return?
let x = if true { 42 }
print(x)
// Expected: 42 or error (may require else for expression use)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

/// If/else with mismatched branch types -- dynamic typing allows it.
#[test]
fn cf_22_if_else_type_mismatch() {
    let code = r#"
// Test 22: If/else with mismatched types
let x = if true { 42 } else { "hello" }
print(x)
// Expected: error or dynamic typing allows it
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

// =========================================================================
// If as statement (not expression)
// =========================================================================

/// If used purely as a statement, not capturing return value.
#[test]
fn cf_23_if_statement_not_expression() {
    let code = r#"
// Test 23: If used purely as statement (not capturing value)
let x = 10
if x > 5 {
  print("big")
}
if x < 5 {
  print("small")
}
print("done")
// Expected: big done
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("big\ndone");
}

/// If without else used as a statement (not as an expression).
#[test]
fn cf_36_if_no_else_statement() {
    let code = r#"
// Test 36: If without else used as statement (no assignment)
let x = 5
if x > 3 {
  print("greater")
}
print("done")
// Expected: greater, done
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("greater\ndone");
}

/// If without else, condition false, used as expression -- returns unit.
#[test]
fn cf_37_if_no_else_false() {
    let code = r#"
// Test 37: If without else, condition false, used as expression
let x = if false { 42 }
print(x)
// Expected: () or None
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("()");
}

// =========================================================================
// If expression in print / function call
// =========================================================================

/// An if expression used directly as a print argument.
#[test]
fn cf_31_if_expression_in_print() {
    let code = r#"
// Test 31: If expression used directly in print (not assigned)
print(if true { "yes" } else { "no" })
// Expected: yes or syntax error
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("yes");
}

/// A block expression used as the condition of an if statement.
#[test]
fn cf_27_block_in_if() {
    let code = r#"
// Test 27: Block expression inside if condition
let x = 10
if { let t = x * 2; t > 15 } {
  print("yes")
} else {
  print("no")
}
// Expected: yes (if block-as-condition supported) or syntax error
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("yes");
}
